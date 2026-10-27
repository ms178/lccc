#!/usr/bin/env python3
"""Restore the GitHub-only agent toolchain cache from an immutable tag.

The publisher stores every file as ``<name>.partNN`` blobs in an orphan tag.
This tool reassembles those blobs, verifies each canonical file against the
tag's MANIFEST.sha256 (and CACHE.json against the reviewed in-tree lock),
extracts the toolchains, and writes wrappers plus ``env.sh``.  No network host
other than the Git remote is contacted.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_LOCK = ROOT / 'engineering/toolchains/agent-cache-lock.json'
DEFAULT_TAG = 'harness-cache-7'


def run(cmd: list[str], *, cwd: Path | None = None, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, cwd=cwd, check=check, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, text=True)


def git(repo: Path, *args: str) -> str:
    return run(['git', '-C', str(repo), *args]).stdout


def digest(path: Path) -> str:
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def materialise(repo: Path, tag: str, name: str, dest: Path, parts: int) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    with dest.open('wb') as out:
        for index in range(parts):
            member = f'{name}.part{index:02d}'
            raw = subprocess.run(['git', '-C', str(repo), 'cat-file', 'blob', f'{tag}:{member}'],
                                 check=True, stdout=subprocess.PIPE).stdout
            out.write(raw)


def read_blob(repo: Path, tag: str, member: str) -> str:
    return run(['git', '-C', str(repo), 'show', f'{tag}:{member}']).stdout


def parse_manifest(text: str) -> dict[str, str]:
    out: dict[str, str] = {}
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith('#'):
            continue
        parts = line.split(None, 1)
        if len(parts) == 2:
            out[parts[1].lstrip('*')] = parts[0]
    return out


def safe_extract(archive: Path, prefix: Path) -> None:
    with tarfile.open(archive) as tar:
        for member in tar.getmembers():
            name = member.name.lstrip('./')
            if not name or name.startswith('/') or '..' in Path(name).parts:
                raise RuntimeError(f'unsafe tar member {member.name!r} in {archive.name}')
    subprocess.run(['tar', '-xf', str(archive), '-C', str(prefix), '--no-same-owner'], check=True)


def write_wrapper(path: Path, target: Path, lib_dirs: list[str]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    libs = ':'.join(str(p) for p in lib_dirs)
    body = f'''#!/bin/sh
export LD_LIBRARY_PATH="{libs}${{LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}}"
exec "{target}" "$@"
'''
    tmp = path.with_suffix(path.suffix + '.tmp')
    tmp.write_text(body)
    tmp.chmod(0o755)
    tmp.replace(path)


def build_env(prefix: Path, cache: dict, lib_dirs: list[str], shim_dir: Path) -> None:
    tools = cache['tools']
    by_name = {name: spec for name, spec in tools.items()}
    cc = shim_dir / 'gcc-16' if (shim_dir / 'gcc-16').exists() else ''
    cxx = shim_dir / 'g++-16' if (shim_dir / 'g++-16').exists() else ''
    clang = shim_dir / 'clang-23' if (shim_dir / 'clang-23').exists() else shim_dir / 'clang'
    rustc = shim_dir / 'rustc' if (shim_dir / 'rustc').exists() else ''
    mold = shim_dir / 'mold' if (shim_dir / 'mold').exists() else ''
    lines = [
        f'export LCCC_AGENT_CACHE_PREFIX="{prefix}"',
        f'export PATH="{shim_dir}:$PATH"',
        f'export LD_LIBRARY_PATH="{":".join(str(p) for p in lib_dirs)}${{LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}}"',
    ]
    for var, val in (('CC', cc), ('CXX', cxx), ('CLANG', clang), ('RUSTC', rustc),
                     ('CARGO', shim_dir / 'cargo' if (shim_dir / 'cargo').exists() else ''),
                     ('MOLD', mold), ('LCCC_QEMU_PREFIX', prefix / tools.get('qemu', {}).get('root', 'qemu-user-11.1.2-x86_64'))):
        if val:
            lines.append(f'export {var}="{val}"')
    (prefix / 'env.sh').write_text('\n'.join(lines) + '\n')


def restore(repo: Path, remote: str, tag: str, prefix: Path, lock_path: Path | None,
            *, skip_lock: bool, only: set[str] | None, no_shims: bool,
            verify_only: bool, force: bool) -> int:
    lock = None
    if lock_path and lock_path.is_file() and not skip_lock:
        lock = json.loads(lock_path.read_text())
        if lock.get('tag') != tag:
            raise RuntimeError(f'lock names tag {lock.get("tag")!r}, requested {tag!r}')
    if remote:
        run(['git', '-C', str(repo), 'fetch', '--no-tags', remote,
             f'+refs/tags/{tag}:refs/tags/{tag}'])
    tag_sha = git(repo, 'rev-parse', tag).strip()
    if lock and lock.get('tag_sha') and tag_sha != lock['tag_sha']:
        raise RuntimeError(f'tag {tag} resolved to {tag_sha}, lock expects {lock["tag_sha"]}')
    parts_text = read_blob(repo, tag, 'PARTS.tsv.part00')
    parts: dict[str, int] = {}
    for line in parts_text.splitlines():
        if not line.strip():
            continue
        n, name = line.split('\t', 1)
        parts[name] = int(n)
    required = {'MANIFEST.sha256', 'CACHE.json', 'PARTS.tsv', 'README.md', 'debian-debs.sha256'}
    missing = [n for n in required if n not in parts]
    if missing:
        raise RuntimeError(f'tag {tag} is missing metadata: {missing}')
    work = Path(tempfile.mkdtemp(prefix='lccc-agent-cache-'))
    try:
        for name, count in parts.items():
            materialise(repo, tag, name, work / name, count)
        manifest = parse_manifest((work / 'MANIFEST.sha256').read_text())
        cache = json.loads((work / 'CACHE.json').read_text())
        if lock and lock.get('cache_json_sha256'):
            if digest(work / 'CACHE.json') != lock['cache_json_sha256']:
                raise RuntimeError('CACHE.json digest disagrees with the reviewed lock')
        for name, want in manifest.items():
            if name == 'CACHE.json':
                continue
            got = digest(work / name)
            if got != want:
                raise RuntimeError(f'SHA-256 mismatch for {name}: {got} != {want}')
        tools = cache['tools']
        selected = set(tools) if not only else set(only)
        unknown = selected - set(tools)
        if unknown:
            raise RuntimeError(f'unknown --only tools: {sorted(unknown)}')
        print(f'agent cache {tag}: verified {len(manifest)} files, tag {tag_sha[:12]}')
        if verify_only:
            return 0
        prefix = prefix.resolve()
        prefix.mkdir(parents=True, exist_ok=True)
        stamp = prefix / '.lccc-agent-cache'
        stamp_value = f'{tag_sha} {digest(work / "CACHE.json")}\n'
        if stamp.is_file() and stamp.read_text() == stamp_value and not force:
            print(f'agent cache already restored in {prefix}; use --force to redo')
            return 0
        for name in sorted(selected, key=lambda n: cache.get('priority', []).index(n)
                           if n in cache.get('priority', []) else 999):
            spec = tools[name]
            archive = work / spec['file']
            print(f'extracting {name} {spec["version"]}: {spec["file"]}')
            safe_extract(archive, prefix)
        shim_dir = prefix / 'bin'
        lib_dirs: list[str] = []
        if not no_shims:
            shim_dir.mkdir(parents=True, exist_ok=True)
            # Collect library paths first, so every wrapper can set them even
            # when the caller has not sourced env.sh.
            for name in cache.get('priority', list(tools)):
                if name not in selected:
                    continue
                root = prefix / tools[name]['root']
                for candidate in (root / 'lib', root / 'lib64', root / 'usr/lib',
                                  root / 'usr/lib/x86_64-linux-gnu'):
                    if candidate.is_dir() and str(candidate) not in lib_dirs:
                        lib_dirs.append(str(candidate))
            # Priority order keeps current Rust/Clang ahead of the MSRV floor.
            for name in cache.get('priority', list(tools)):
                if name not in selected:
                    continue
                root = prefix / tools[name]['root']
                for candidate in (root / 'bin', root / 'usr/bin'):
                    if candidate.is_dir():
                        for entry in sorted(candidate.iterdir()):
                            if entry.is_dir() or not (entry.stat().st_mode & 0o111):
                                continue
                            target = shim_dir / entry.name
                            if not target.exists():
                                write_wrapper(target, entry, lib_dirs)
            build_env(prefix, cache, lib_dirs, shim_dir)
            stamp.write_text(stamp_value)
            (prefix / 'README.txt').write_text(
                f'GitHub-only agent toolchain cache {tag}\n'
                f'tag_sha: {tag_sha}\n'
                f'Source: {work}/CACHE.json was verified against MANIFEST.sha256.\n'
                'Run `. env.sh` to put the pinned tools on PATH.\n')
        return 0
    finally:
        shutil.rmtree(work, ignore_errors=True)


def self_test() -> int:
    """Build a tiny fake tag and prove reassembly, verification and extraction."""
    with tempfile.TemporaryDirectory() as td:
        base = Path(td)
        repo = base / 'repo'
        repo.mkdir()
        run(['git', 'init', '-q', '-b', 'main', str(repo)])
        run(['git', '-C', str(repo), 'config', 'user.name', 'agent-cache-test'])
        run(['git', '-C', str(repo), 'config', 'user.email', 'test@example.invalid'])
        stage = base / 'stage'
        stage.mkdir()
        tools = {}
        files = {}
        for name, version, root, script in (
            ('gcc', '16.2.0', 'gcc-16', '#!/bin/sh\necho "gcc-16 (GCC) 16.2.0"\n'),
            ('binutils', '2.47.50.20261021-1', 'binutils-2.47-debian-sid-x86_64', '#!/bin/sh\necho "GNU assembler 2.47"\n'),
        ):
            r = stage / root
            (r / 'bin').mkdir(parents=True)
            (r / 'bin' / ('gcc-16' if name == 'gcc' else 'as')).write_text(script)
            (r / 'bin' / ('gcc-16' if name == 'gcc' else 'as')).chmod(0o755)
            archive = base / f'{root}.tar.xz'
            subprocess.run(['tar', '--sort=name', '--mtime=@0', '--owner=0', '--group=0',
                            '--numeric-owner', '-I', 'xz -T2 -6', '-cf', str(archive),
                            '-C', str(stage), root], check=True)
            files[archive.name] = archive.read_bytes()
            tools[name] = {'version': version, 'file': archive.name,
                           'sha256': digest(archive), 'size': archive.stat().st_size,
                           'root': root, 'kind': 'tar.xz'}
        cache = {'schema': 1, 'repository': 'ms178/lccc', 'tag': 'agent-selftest',
                 'part_size': 90 * 1024 * 1024, 'tools': tools,
                 'priority': ['gcc', 'binutils'],
                 'provenance': {'source': 'synthetic self-test'}}
        files['CACHE.json'] = (json.dumps(cache, indent=2, sort_keys=True) + '\n').encode()
        files['MANIFEST.sha256'] = ''.join(f"{v['sha256']}  {v['file']}\n" for v in tools.values()).encode()
        files['README.md'] = b'# self-test\n'
        files['debian-debs.sha256'] = b'# synthetic\n'
        parts = {name: 1 for name in files}
        parts['PARTS.tsv'] = 1
        files['PARTS.tsv'] = ''.join(f'1\t{name}\n' for name in sorted(parts)).encode()
        for name, data in files.items():
            (stage / f'{name}.part00').write_bytes(data)
        lines = []
        for p in sorted(stage.glob('*.part00')):
            sha = run(['git', '-C', str(repo), 'hash-object', '-w', str(p)]).stdout.strip()
            lines.append(f'100644 blob {sha}\t{p.name}')
        proc = subprocess.run(['git', '-C', str(repo), 'mktree'], input='\n'.join(lines) + '\n',
                              text=True, stdout=subprocess.PIPE, check=True)
        tree_sha = proc.stdout.strip()
        commit = run(['git', '-C', str(repo), 'commit-tree', tree_sha, '-m', 'self-test']).stdout.strip()
        run(['git', '-C', str(repo), 'tag', '-a', 'agent-selftest', commit, '-m', 'self-test'])
        prefix = base / 'prefix'
        rc = restore(repo, '', 'agent-selftest', prefix, None, skip_lock=True,
                     only=None, no_shims=False, verify_only=False, force=False)
        if rc != 0:
            return rc
        if not (prefix / 'bin' / 'gcc-16').is_file() or not (prefix / 'bin' / 'as').is_file():
            raise AssertionError('self-test did not create wrappers')
        if 'LCCC_AGENT_CACHE_PREFIX' not in (prefix / 'env.sh').read_text():
            raise AssertionError('self-test did not write env.sh')
        print('agent cache restore self-test: PASS')
        return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tag', default=DEFAULT_TAG)
    parser.add_argument('--prefix', type=Path,
                        default=Path(os.environ.get('LCCC_AGENT_CACHE_PREFIX',
                                                   str(Path.home() / '.cache/lccc/agent-toolchains'))))
    parser.add_argument('--repo', type=Path, default=ROOT)
    parser.add_argument('--remote', default='origin')
    parser.add_argument('--lock', type=Path, default=DEFAULT_LOCK)
    parser.add_argument('--only', default='', help='comma-separated tool names to restore')
    parser.add_argument('--no-shims', action='store_true')
    parser.add_argument('--verify-only', action='store_true')
    parser.add_argument('--skip-lock', action='store_true')
    parser.add_argument('--force', action='store_true')
    parser.add_argument('--self-test', action='store_true')
    args = parser.parse_args(argv)
    if args.self_test:
        return self_test()
    only = {x.strip() for x in args.only.split(',') if x.strip()} or None
    return restore(args.repo.resolve(), args.remote, args.tag, args.prefix,
                   args.lock if args.lock.is_absolute() else (ROOT / args.lock),
                   skip_lock=args.skip_lock, only=only, no_shims=args.no_shims,
                   verify_only=args.verify_only, force=args.force)


if __name__ == '__main__':
    raise SystemExit(main())
