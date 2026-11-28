#!/usr/bin/env python3
"""Opt-in, pinned LLVM 23.1.6 research slice: Clang, LLD and x86-64 ASan/UBSan.

The reviewed agent cache/CI Clang 23.1.5 pin is NOT changed. This installs a
separate current-release oracle, without building LLVM or unpacking its entire
multi-GiB distribution. Requires zstd and a working host C linker/libc.

  python3 scripts/restore_llvm_oracle_slice.py --prefix /large/drive/llvm-23.1.6

--archive reuses a downloaded official archive (the same size/SHA checks apply).
No system directories are modified, and an unverified existing prefix is never
overlaid. The whole archive is verified before streaming selected regular files
into a private staging directory. Publication follows compiler/header/runtime
probes. Warm reuse checks every installed file against its private receipt;
this detects damage, not an attacker controlling both the prefix and receipt.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request

from restore_agent_toolchain_cache import validate_restore_prefix

VERSION = '23.1.6'
TOP = f'LLVM-{VERSION}-Linux-X64'
URL = f'https://github.com/llvm/llvm-project/releases/download/llvmorg-{VERSION}/{TOP}.tar.zst'
# GitHub release metadata, published 2026-11-17, checked 2026-11-27.
SHA256 = '59e14234f145927f982b50627e8bafaf2fca50151b7d973d6a0b40796ddcfe9a'
ARCHIVE_SIZE = 1198414275
RECEIPT = '.lccc-llvm-slice.json'
BINARIES = {'bin/clang', 'bin/clang-23', 'bin/lld', 'bin/lld-23', 'bin/ld.lld'}


def selected_path(name: str) -> str | None:
    if not name.startswith(TOP + '/'):
        return None
    relative = name[len(TOP) + 1:]
    path = PurePosixPath(relative)
    if path.is_absolute() or '..' in path.parts:
        raise ValueError(f'unsafe archive member: {name!r}')
    if relative in BINARIES or relative.startswith('lib/clang/23/include/'):
        return relative
    # LLVM uses both layouts. New releases omit the -x86_64 suffix and put
    # the triple in the directory instead; selecting only lib/linux silently
    # left a seemingly installed Clang without its sanitizer link inputs.
    modern = re.fullmatch(r'lib/clang/23/lib/x86_64-unknown-linux-gnu/libclang_rt\.(asan|ubsan)[^/]*\.(a|so)', relative)
    legacy = re.fullmatch(r'lib/clang/23/lib/linux/libclang_rt\.(asan|ubsan)[^/]*-x86_64\.(a|so)', relative)
    return relative if modern or legacy else None


def digest(path: Path) -> str:
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def extract_members(tar: tarfile.TarFile, stage: Path) -> list[str]:
    selected = []
    for member in tar:
        relative = selected_path(member.name)
        if relative is None or member.isdir():
            continue
        if not member.isfile():
            # Only the well-known driver aliases are recreated, not arbitrary
            # archive links. Header/runtime links require a layout review.
            if relative in BINARIES and (member.issym() or member.islnk()):
                continue
            raise ValueError(f'nonregular selected archive member: {member.name!r}')
        destination = stage / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        stream = tar.extractfile(member)
        if stream is None:
            raise ValueError(f'missing archive payload: {member.name!r}')
        with stream, destination.open('xb') as output:
            shutil.copyfileobj(stream, output, 1 << 20)
        destination.chmod(member.mode & 0o755)
        selected.append(relative)
    return selected


def checked(args: list[str], **kwargs) -> str:
    result = subprocess.run(args, capture_output=True, text=True, timeout=120, **kwargs)
    if result.returncode:
        raise RuntimeError(f'{args[0]} failed: {result.stderr.strip()}')
    return result.stdout.strip()


def probe(prefix: Path, work: Path) -> dict:
    clang, lld = prefix / 'bin/clang', prefix / 'bin/ld.lld'
    version = checked([str(clang), '-dumpversion'])
    linker = checked([str(lld), '--version'])
    if version != VERSION or not linker.startswith(f'LLD {VERSION} '):
        raise ValueError('LLVM slice version mismatch')
    source = work / 'probe.c'
    source.write_text('#include <stddef.h>\n#include <stdint.h>\n'
                      'int main(int n, char **p) { (void)p; volatile int v = n; '
                      'return v * 3 != 3; }\n')
    binary = work / 'probe'
    checked([str(clang), '-O1', '-fsanitize=address,undefined',
             '-fno-sanitize-recover=all', str(source), '-o', str(binary)])
    checked([str(binary)], env={**os.environ, 'ASAN_OPTIONS': 'detect_leaks=0',
                               'UBSAN_OPTIONS': 'halt_on_error=1'})
    return {'clang': version, 'lld': linker, 'asan_ubsan_link_and_run': 'pass'}


def install(prefix: Path, archive: Path | None = None) -> dict:
    prefix = validate_restore_prefix(prefix)
    if prefix.exists():
        record = json.loads((prefix / RECEIPT).read_text())
        if record.get('version') != VERSION or record.get('archive_sha256') != SHA256:
            raise ValueError('unverified existing prefix; choose a fresh private directory')
        files, aliases = record.get('files'), record.get('aliases')
        if not isinstance(files, dict) or not files or not isinstance(aliases, dict):
            raise ValueError('incomplete LLVM slice receipt')
        for tool in ('bin/clang', 'bin/ld.lld'):
            backing = (prefix / tool).resolve(strict=True).relative_to(prefix).as_posix()
            if backing not in files:
                raise ValueError('LLVM driver has no verified backing file')
        for relative, want in files.items():
            path = prefix / relative
            if (selected_path(TOP + '/' + relative) != relative or path.is_symlink()
                    or not path.resolve().is_relative_to(prefix) or digest(path) != want):
                raise ValueError('installed LLVM slice changed; choose a fresh prefix')
        allowed_aliases = {'bin/clang': ('clang-23',), 'bin/ld.lld': ('lld', 'lld-23')}
        for alias, target in aliases.items():
            if target not in allowed_aliases.get(alias, ()) or os.readlink(prefix / alias) != target:
                raise ValueError('installed LLVM driver alias changed')
        with tempfile.TemporaryDirectory(prefix='llvm-probe-') as td:
            probe(prefix, Path(td))
        return record
    if shutil.which('zstd') is None:
        raise ValueError('zstd is required (install it before restoring the slice)')
    prefix.parent.mkdir(parents=True, exist_ok=True)
    needed = (ARCHIVE_SIZE if archive is None else 0) + 768 * 1024 * 1024
    if shutil.disk_usage(prefix.parent).free < needed:
        raise ValueError(f'need {needed // (1 << 20)} MiB free for archive and selected tools')
    with tempfile.TemporaryDirectory(prefix='.llvm-slice-', dir=prefix.parent) as td:
        work = Path(td)
        if archive is None:
            archive = work / 'release.tar.zst'
            print(f'downloading {URL}', flush=True)
            with urllib.request.urlopen(URL, timeout=120) as response, archive.open('xb') as out:
                shutil.copyfileobj(response, out, 1 << 20)
        if archive.stat().st_size != ARCHIVE_SIZE or digest(archive) != SHA256:
            raise ValueError('LLVM release archive size/SHA-256 mismatch')
        stage = work / 'install'
        stage.mkdir()
        decoder = subprocess.Popen(['zstd', '-dc', '--long=31', str(archive)], stdout=subprocess.PIPE)
        try:
            with tarfile.open(fileobj=decoder.stdout, mode='r|') as tar:
                selected = extract_members(tar, stage)
        finally:
            decoder.stdout.close()
            status = decoder.wait(timeout=60)
        if status:
            raise RuntimeError(f'zstd exited {status}')
        aliases = {}
        for alias, choices in [('bin/clang', ('clang-23',)),
                               ('bin/ld.lld', ('lld', 'lld-23'))]:
            if not (stage / alias).exists():
                target = next((name for name in choices if (stage / 'bin' / name).is_file()), None)
                if target is None:
                    raise ValueError(f'archive lacks the driver backing {alias}')
                (stage / alias).symlink_to(target)
                aliases[alias] = target
        identities = probe(stage, work)
        record = {'version': VERSION, 'url': URL, 'archive_sha256': SHA256,
                  'files': {name: digest(stage / name) for name in selected},
                  'aliases': aliases, 'probes': identities,
                  'scope': 'optional research oracle; existing agent-cache/CI pins unchanged'}
        (stage / RECEIPT).write_text(json.dumps(record, indent=2, sort_keys=True) + '\n')
        os.replace(stage, prefix)
    return record


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prefix', type=Path, required=True)
    parser.add_argument('--archive', type=Path)
    args = parser.parse_args()
    try:
        record = install(args.prefix, args.archive)
        print(json.dumps({'prefix': str(args.prefix), 'version': VERSION,
                          'files': len(record['files']), 'probes': record['probes']}, indent=2))
        return 0
    except (OSError, ValueError, KeyError, RuntimeError, tarfile.TarError,
            subprocess.TimeoutExpired) as error:
        parser.exit(2, f'LLVM slice: {error}\n')


if __name__ == '__main__':
    raise SystemExit(main())
