#!/usr/bin/env python3
"""Restore the reviewed Rust/Clang cache lock, never a remote self-signed hash."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
# Older unpatched Python releases lack the security extraction filter. Never
# silently fall back to unrestricted extraction on those hosts.
if not hasattr(tarfile, 'data_filter'):
    try:
        from backports import tarfile
    except ImportError as error:
        raise RuntimeError('Use a patched Python with tarfile.data_filter (or install backports.tarfile)') from error
import tempfile

LOCK = Path(__file__).resolve().parent.parent / 'engineering/toolchains/cache-lock.json'
VERSIONS = {'rust': '1.99.1', 'clang': '23.1.4'}


def validate(entry, name):
    if entry['version'] != VERSIONS[name]:
        raise ValueError(f'{name}: unexpected cache version')
    if not re.fullmatch(r'[0-9a-f]{64}', entry['sha256']):
        raise ValueError(f'{name}: malformed SHA-256')
    expected = (f'https://github.com/ms178/lccc/releases/download/harness-cache-4/'
                f'{name}-{entry["version"]}-{entry["sha256"]}.tar.xz')
    if entry['url'] != expected:
        raise ValueError(f'{name}: untrusted cache URL')


def verify(root, name):
    command = [str(root / ('bin/rustc' if name == 'rust' else 'bin/clang')),
               '--version' if name == 'rust' else '-dumpversion']
    output = subprocess.check_output(command, text=True).strip()
    actual = output.split()[1] if name == 'rust' else output
    if actual != VERSIONS[name]:
        raise ValueError(f'{name}: installed identity {output!r} is not {VERSIONS[name]}')
    if name == 'rust':
        for binary in ('cargo', 'rustfmt', 'clippy-driver'):
            subprocess.run([str(root / 'bin' / binary), '--version'], check=True)


def unpack(archive, destination):
    # One packaging directory is removed; data_filter rejects path traversal,
    # absolute symlinks and special devices, including links escaping the root.
    with tarfile.open(archive) as source:
        members = []
        for member in source:
            top, separator, rest = member.name.partition('/')
            if top in ('', '.', '..') or top.startswith('/'):
                raise ValueError('invalid archive root')
            if not separator or not rest:
                continue
            member.name = rest
            if member.islnk():
                if not member.linkname.startswith(top + '/'):
                    raise ValueError('hard link escapes archive root')
                member.linkname = member.linkname[len(top) + 1:]
            members.append(member)
        source.extractall(destination, members=members, filter='data')


def restore(entry, name, prefix, downloads):
    validate(entry, name)
    root = prefix / f'{name}-{entry["version"]}'
    marker = root / '.archive-sha256'
    if root.exists():
        if not marker.is_file() or marker.read_text().strip() != entry['sha256']:
            raise ValueError(f'{root}: existing directory is not this verified cache; use a fresh prefix')
        verify(root, name)
        return
    archive = downloads / entry['url'].rsplit('/', 1)[1]
    if not archive.exists():
        part = archive.with_suffix('.partial')
        try:
            subprocess.run(['curl', '--fail', '--location', '--retry', '3', '--proto', '=https',
                            '--output', str(part), entry['url']], check=True)
            part.replace(archive)
        finally:
            part.unlink(missing_ok=True)
    with archive.open('rb') as stream:
        actual = hashlib.file_digest(stream, 'sha256').hexdigest()
    if actual != entry['sha256']:
        raise ValueError(f'{archive}: SHA-256 mismatch; refusing extraction')
    temporary = Path(tempfile.mkdtemp(prefix=f'.{name}-', dir=prefix))
    try:
        unpack(archive, temporary)
        verify(temporary, name)
        (temporary / '.archive-sha256').write_text(actual + '\n')
        temporary.rename(root)
    finally:
        if temporary.exists():
            shutil.rmtree(temporary)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prefix', type=Path, required=True)
    parser.add_argument('--tool', choices=['all', *VERSIONS], default='all')
    parser.add_argument('--lock', type=Path, default=LOCK)
    args = parser.parse_args()
    lock = json.loads(args.lock.read_text())
    if lock.get('schema') != 1 or lock.get('repository') != 'ms178/lccc' or lock.get('tag') != 'harness-cache-4':
        raise ValueError('unknown cache lock schema or transport')
    args.prefix.mkdir(parents=True, exist_ok=True)
    downloads = args.prefix / 'dl'
    downloads.mkdir(exist_ok=True)
    for name in VERSIONS if args.tool == 'all' else [args.tool]:
        restore(lock['tools'][name], name, args.prefix, downloads)


if __name__ == '__main__':
    main()
