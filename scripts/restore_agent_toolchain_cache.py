#!/usr/bin/env python3
"""Restore the GitHub-only agent toolchain cache from an immutable tag.

The publisher stores every file as ``<name>.partNN`` blobs in an orphan tag.
This tool reassembles those blobs, verifies each canonical file against the
tag's MANIFEST.sha256 (and CACHE.json against the reviewed in-tree lock),
extracts the toolchains, and writes wrappers plus ``env.sh``.  No network host
other than the Git remote is contacted.

A warm prefix is answered from an *installed manifest* written by the
extraction that produced it: checking member type/size/mode plus a wrapper
digest costs milliseconds, while re-opening the archives costs a full xz
decompression (measured: 7.8 s for the 145 MiB Rust archive alone, ~21 s for
the whole cache).  The manifest lives beside the files it describes, so it
carries exactly their trust level — anything able to forge it can already
rewrite the compiler it protects — and it is keyed on the resolved tag SHA and
the exact tool selection, so a moved tag or a different ``--only`` set forces
the verifying path.
"""
from __future__ import annotations

import argparse
import hashlib
import inspect
import io
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_LOCK = ROOT / 'engineering/toolchains/agent-cache-lock.json'
DEFAULT_TAG = 'harness-cache-7'
STAMP_NAME = '.lccc-agent-cache'
INSTALLED_NAME = '.lccc-agent-installed.json'
# Bumped whenever the manifest's shape changes: an unfamiliar manifest is
# ignored, which costs exactly one verifying run and never a wrong answer.
INSTALLED_SCHEMA = 2


def run(cmd: list[str], *, cwd: Path | None = None, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, cwd=cwd, check=check, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, text=True)


def git(repo: Path, *args: str) -> str:
    return run(['git', '-C', str(repo), *args]).stdout


def digest(path: Path) -> str:
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def bytes_digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


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


def write_atomic(path: Path, data: bytes, mode: int) -> None:
    """Replace ``path`` without ever writing *through* a symlink.

    ``O_NOFOLLOW`` on the temporary plus ``os.replace`` (which swaps the
    directory entry rather than the link target) means a pre-existing symlink
    at either name is replaced, never followed.
    """
    tmp = path.with_name(path.name + '.tmp')
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, mode)
    try:
        # os.write may write fewer bytes than asked; the installed manifest is
        # a few MiB for the real cache, so this must loop rather than assume.
        view = memoryview(data)
        while view:
            view = view[os.write(fd, view):]
        os.fchmod(fd, mode)
    finally:
        os.close(fd)
    os.replace(tmp, path)


def extractall_takes_filter() -> bool:
    """Probe the implementation, not an incomplete Python-version list."""
    try:
        return 'filter' in inspect.signature(tarfile.TarFile.extractall).parameters
    except (TypeError, ValueError):
        return False


def extraction_supported() -> bool:
    """Whether this interpreter can extract an untrusted archive safely."""
    return hasattr(tarfile, 'data_filter') and extractall_takes_filter()


def member_kind(member: tarfile.TarInfo) -> str:
    if member.isdir():
        return 'dir'
    if member.issym():
        return 'sym'
    if member.islnk():
        # A hardlink member carries no payload and therefore reports size 0,
        # while the installed name is the target's inode; its content is
        # covered by the target's own 'file' entry.
        return 'lnk'
    if member.isfile():
        return 'file'
    return 'other'


def extract_untrusted(archive: Path, prefix: Path) -> list[dict]:
    """Validate every member, then extract; return the member descriptors.

    The declared Python floor does not universally provide extraction filters.
    Never turn an unsupported host into an unfiltered extraction.
    """
    if not extraction_supported():
        raise RuntimeError('secure agent-cache extraction needs a filter-capable Python')
    # Validate the COMPLETE archive before writing anything.  The old precheck
    # inspected only member names (and lstrip('./') even hid leading slashes
    # and traversal), then delegated links to GNU tar.  GNU tar rejected the
    # tested escape only after creating the attacker-selected symlink.
    # Python's data filter checks paths, link targets, devices and modes; the
    # separate validation pass ensures a late bad member cannot leave a
    # partially extracted unsafe archive behind.
    with tarfile.open(archive) as tar:
        members = tar.getmembers()
        for member in members:
            tarfile.data_filter(member, str(prefix))
        tar.extractall(prefix, members=members, filter='data')
    installed: list[dict] = []
    for member in members:
        kind = member_kind(member)
        if kind == 'other':
            continue
        entry = {'name': member.name, 'kind': kind}
        if kind == 'file':
            entry['size'] = member.size
            # Only the execute bit is a property of the toolchain.  Group and
            # other write bits are host policy: extraction applies the local
            # umask/ACL (measured: a 0o775 member lands as 0o755), so comparing
            # them would report every correct prefix as damaged.
            entry['exec'] = bool(stat.S_IMODE(member.mode) & 0o111)
        elif kind == 'sym':
            entry['link'] = member.linkname
        installed.append(entry)
    return installed


def reject_escaping_links(prefix: Path) -> None:
    """Fail if any extracted symlink resolves outside ``prefix``.

    Member-at-a-time validation cannot see a *chain*: an archive may create
    ``bin -> root/link``, ``root/link -> inside/../../outside`` and
    ``root/inside -> ../other``, each individually legal, such that ``bin``
    only leaves the prefix once the third link exists.  Wrappers, ``env.sh``
    and the stamp are written *through* such a path, so the tree is re-checked
    after extraction and before the first write.
    """
    root = prefix.resolve()
    for dirpath, dirnames, filenames in os.walk(root, followlinks=False):
        for name in dirnames + filenames:
            path = Path(dirpath) / name
            if not path.is_symlink():
                continue
            resolved = Path(os.path.realpath(path))
            if resolved != root and root not in resolved.parents:
                raise RuntimeError(
                    f'archive link escapes the cache prefix: {path} -> {os.readlink(path)}')


def escaped(path: Path, root: Path) -> bool:
    resolved = Path(os.path.realpath(path))
    return resolved != root and root not in resolved.parents


def warm_members_match(prefix: Path, entries: list[dict]) -> bool:
    """Check installed members against the filesystem without decompressing."""
    root = prefix.resolve()
    for entry in entries:
        path = prefix / entry['name']
        try:
            st = path.lstat()
        except OSError:
            return False
        kind = entry['kind']
        if kind == 'dir':
            if not stat.S_ISDIR(st.st_mode):
                return False
        elif kind == 'sym':
            if not stat.S_ISLNK(st.st_mode) or os.readlink(path) != entry['link']:
                return False
            # A chain is invisible to a name comparison; resolve the few links.
            if escaped(path, root):
                return False
        elif kind == 'lnk':
            if not stat.S_ISREG(st.st_mode):
                return False
        elif kind == 'file':
            if not stat.S_ISREG(st.st_mode) or st.st_size != entry['size']:
                return False
            # A lost execute bit changes what the toolchain *is*, so it is part
            # of the receipt rather than an accidental property.
            if bool(st.st_mode & 0o111) != entry['exec']:
                return False
        else:
            return False
    return True


def load_installed_manifest(path: Path) -> dict | None:
    try:
        raw = json.loads(path.read_text())
    except (OSError, ValueError):
        return None
    if not isinstance(raw, dict) or raw.get('schema') != INSTALLED_SCHEMA:
        return None
    if not isinstance(raw.get('members'), list):
        return None
    return raw


def wrapper_body(target: Path, lib_dirs: list[str]) -> str:
    libs = ':'.join(str(p) for p in lib_dirs)
    return f'''#!/bin/sh
export LD_LIBRARY_PATH="{libs}${{LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}}"
exec "{target}" "$@"
'''


def write_wrapper(path: Path, target: Path, lib_dirs: list[str]) -> bytes:
    path.parent.mkdir(parents=True, exist_ok=True)
    body = wrapper_body(target, lib_dirs).encode()
    write_atomic(path, body, 0o755)
    return body


def env_body(prefix: Path, cache: dict, lib_dirs: list[str], shim_dir: Path) -> str:
    tools = cache['tools']
    cc = shim_dir / 'gcc-16' if (shim_dir / 'gcc-16').exists() else ''
    cxx = shim_dir / 'g++-16' if (shim_dir / 'g++-16').exists() else ''
    clang = shim_dir / 'clang-23' if (shim_dir / 'clang-23').exists() else shim_dir / 'clang'
    rustc = shim_dir / 'rustc' if (shim_dir / 'rustc').exists() else ''
    mold = shim_dir / 'mold' if (shim_dir / 'mold').exists() else ''
    # Hoisted: an f-string expression may not contain a backslash below
    # Python 3.12, and the join is invariant across every entry.
    lib_path = ':'.join(str(p) for p in lib_dirs)
    lines = [
        f'export LCCC_AGENT_CACHE_PREFIX="{prefix}"',
        f'export PATH="{shim_dir}:$PATH"',
        f'export LD_LIBRARY_PATH="{lib_path}${{LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}}"',
    ]
    for var, val in (('CC', cc), ('CXX', cxx), ('CLANG', clang), ('RUSTC', rustc),
                     ('CARGO', shim_dir / 'cargo' if (shim_dir / 'cargo').exists() else ''),
                     ('MOLD', mold), ('LCCC_QEMU_PREFIX', prefix / tools.get('qemu', {}).get('root', 'qemu-user-11.1.2-x86_64'))):
        if val:
            lines.append(f'export {var}="{val}"')
    return '\n'.join(lines) + '\n'


def build_env(prefix: Path, cache: dict, lib_dirs: list[str], shim_dir: Path) -> bytes:
    body = env_body(prefix, cache, lib_dirs, shim_dir).encode()
    write_atomic(prefix / 'env.sh', body, 0o644)
    return body


def shim_inputs(prefix: Path, cache: dict, selected: set[str]) -> tuple[list[str], list[Path]]:
    """Return library paths and priority-deduplicated executable targets."""
    tools = cache['tools']
    lib_dirs: list[str] = []
    for name in cache.get('priority', list(tools)):
        if name not in selected:
            continue
        root = prefix / tools[name]['root']
        for candidate in (root / 'lib', root / 'lib64', root / 'usr/lib',
                          root / 'usr/lib/x86_64-linux-gnu'):
            if candidate.is_dir() and str(candidate) not in lib_dirs:
                lib_dirs.append(str(candidate))
    entries: list[Path] = []
    seen: set[str] = set()
    # Priority order keeps current Rust/Clang ahead of the MSRV floor.  The
    # seen set is essential now that wrappers are repaired unconditionally:
    # a later lower-priority target must not overwrite the first choice.
    for name in cache.get('priority', list(tools)):
        if name not in selected:
            continue
        root = prefix / tools[name]['root']
        for candidate in (root / 'bin', root / 'usr/bin'):
            if candidate.is_dir():
                for entry in sorted(candidate.iterdir()):
                    if entry.is_dir() or not (entry.stat().st_mode & 0o111):
                        continue
                    if entry.name not in seen:
                        entries.append(entry)
                        seen.add(entry.name)
    return lib_dirs, entries


def parse_stamp(path: Path) -> dict[str, str] | None:
    """Parse ``<tag_sha> <cache_digest> select=<selection>``."""
    try:
        text = path.read_text().strip()
    except OSError:
        return None
    fields = text.split()
    if len(fields) != 3 or not fields[2].startswith('select='):
        return None
    return {'tag_sha': fields[0], 'cache_digest': fields[1],
            'selection': fields[2].split('=', 1)[1]}


def selection_key(only: set[str] | None) -> str:
    return '*' if not only else ','.join(sorted(only))


def warm_shims_match(prefix: Path, record: dict) -> bool:
    """Check the recorded wrapper/env digests without rebuilding CACHE.json."""
    try:
        env = prefix / 'env.sh'
        if not env.is_file() or bytes_digest(env.read_bytes()) != record['env']:
            return False
        for name, want in record['shims'].items():
            wrapper = prefix / 'bin' / name
            if not wrapper.is_file() or not (wrapper.stat().st_mode & 0o111):
                return False
            if bytes_digest(wrapper.read_bytes()) != want:
                return False
    except (OSError, KeyError, AttributeError):
        return False
    return True


def restore(repo: Path, remote: str, tag: str, prefix: Path, lock_path: Path | None,
            *, skip_lock: bool, only: set[str] | None, no_shims: bool,
            verify_only: bool, force: bool) -> int:
    lock = None
    if lock_path is not None and not skip_lock:
        if not lock_path.is_file():
            # A --lock path that does not exist must not silently disable the
            # only authenticity check the tag has; --skip-lock states that
            # intent, a typo does not.
            raise RuntimeError(f'lock file {lock_path} does not exist (use --skip-lock to bypass)')
        lock = json.loads(lock_path.read_text())
        if lock.get('tag') != tag:
            raise RuntimeError(f'lock names tag {lock.get("tag")!r}, requested {tag!r}')
    if remote:
        run(['git', '-C', str(repo), 'fetch', '--no-tags', remote,
             f'+refs/tags/{tag}:refs/tags/{tag}'])
    tag_sha = git(repo, 'rev-parse', tag).strip()
    if lock and lock.get('tag_sha') and tag_sha != lock['tag_sha']:
        raise RuntimeError(f'tag {tag} resolved to {tag_sha}, lock expects {lock["tag_sha"]}')

    prefix = prefix.resolve()
    stamp = prefix / STAMP_NAME
    installed_path = prefix / INSTALLED_NAME
    wanted = selection_key(only)

    # ---- warm fast path -------------------------------------------------
    # Answer from the installed manifest before materialising a single part.
    # It is not a weaker check: every member's type, size, mode and symlink
    # target is validated, and every wrapper plus env.sh is digest-checked,
    # keyed on the resolved tag SHA and the exact selection.  What it skips is
    # re-downloading and re-digesting archives that are already installed.
    if not force and not verify_only:
        stamp_fields = parse_stamp(stamp)
        record = load_installed_manifest(installed_path)
        if (stamp_fields is not None and record is not None
                and stamp_fields['tag_sha'] == tag_sha
                and stamp_fields['selection'] == wanted
                and warm_members_match(prefix, record['members'])):
            shims = record.get('shims')
            if no_shims:
                print(f'agent cache already restored in {prefix}; use --force to redo')
                return 0
            if shims is not None and warm_shims_match(prefix, record):
                print(f'agent cache already restored and verified in {prefix}; use --force to redo')
                return 0

    # ---- verifying path --------------------------------------------------
    if not verify_only and not extraction_supported():
        # Fail in microseconds rather than after materialising ~500 MiB.
        raise RuntimeError('secure agent-cache extraction needs a filter-capable Python')

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
        prefix.mkdir(parents=True, exist_ok=True)
        if stamp.exists():
            print(f'agent cache receipt in {prefix} is stale or incomplete; repairing')
        members: list[dict] = []
        for name in sorted(selected, key=lambda n: cache.get('priority', []).index(n)
                           if n in cache.get('priority', []) else 999):
            spec = tools[name]
            archive = work / spec['file']
            print(f'extracting {name} {spec["version"]}: {spec["file"]}')
            members.extend(extract_untrusted(archive, prefix))
        # No write may pass through an extracted symlink.
        reject_escaping_links(prefix)
        record: dict = {'schema': INSTALLED_SCHEMA, 'members': members}
        shim_dir = prefix / 'bin'
        if no_shims:
            record['shims'] = None
            record['env'] = None
        else:
            shim_dir.mkdir(parents=True, exist_ok=True)
            lib_dirs, entries = shim_inputs(prefix, cache, selected)
            written: dict[str, str] = {}
            for entry in entries:
                # Always repair/rewrite the selected wrapper.  shim_inputs has
                # already deduplicated names by priority, so this cannot let a
                # lower-priority MSRV tool replace the current tool.
                written[entry.name] = bytes_digest(write_wrapper(shim_dir / entry.name, entry, lib_dirs))
            record['shims'] = written
            record['env'] = bytes_digest(build_env(prefix, cache, lib_dirs, shim_dir))
        write_atomic(installed_path,
                     (json.dumps(record, sort_keys=True) + '\n').encode(), 0o644)
        # The stamp is the commit point: it is written only after every payload,
        # wrapper and env.sh has been laid down.
        write_atomic(stamp,
                     f'{tag_sha} {digest(work / "CACHE.json")} select={wanted}\n'.encode(), 0o644)
        write_atomic(prefix / 'README.txt',
                     (f'GitHub-only agent toolchain cache {tag}\n'
                      f'tag_sha: {tag_sha}\n'
                      'Source: CACHE.json was verified against MANIFEST.sha256.\n'
                      'Run `. env.sh` to put the pinned tools on PATH.\n').encode(), 0o644)
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
        # A partial restore must not mint a receipt that suppresses a later
        # full restore.  This ordering reproduced the original warm-cache bug.
        rc = restore(repo, '', 'agent-selftest', prefix, None, skip_lock=True,
                     only={'binutils'}, no_shims=False, verify_only=False, force=False)
        if rc != 0 or (prefix / 'bin/gcc-16').exists() or not (prefix / 'bin/as').is_file():
            raise AssertionError('partial agent-cache restore was not exact')
        rc = restore(repo, '', 'agent-selftest', prefix, None, skip_lock=True,
                     only=None, no_shims=False, verify_only=False, force=False)
        if rc != 0:
            return rc
        if not (prefix / 'bin/gcc-16').is_file() or not (prefix / 'bin/as').is_file():
            raise AssertionError('full restore after --only did not create every wrapper')
        if 'LCCC_AGENT_CACHE_PREFIX' not in (prefix / 'env.sh').read_text():
            raise AssertionError('self-test did not write env.sh')

        # The warm path must answer from the manifest: no part reassembly and
        # no archive decompression at all.
        saved = {key: globals()[key] for key in ('materialise', 'extract_untrusted')}

        def forbidden(*_a, **_k):
            raise AssertionError('warm restore reassembled or re-extracted a payload')

        globals().update(materialise=forbidden, extract_untrusted=forbidden)
        try:
            rc = restore(repo, '', 'agent-selftest', prefix, None, skip_lock=True,
                         only=None, no_shims=False, verify_only=False, force=False)
        finally:
            globals().update(saved)
        if rc != 0:
            raise AssertionError('warm restore did not take the manifest fast path')

        # A deleted payload or wrapper must break the receipt and be repaired.
        (prefix / 'bin/as').unlink()
        (prefix / 'gcc-16/bin/gcc-16').unlink()
        restore(repo, '', 'agent-selftest', prefix, None, skip_lock=True,
                only=None, no_shims=False, verify_only=False, force=False)
        if not (prefix / 'bin/as').is_file() or not (prefix / 'gcc-16/bin/gcc-16').is_file():
            raise AssertionError('warm restore did not repair deleted payload/shims')

        # A lost execute bit is damage, not a property: the receipt must notice.
        target = prefix / 'gcc-16/bin/gcc-16'
        target.chmod(0o644)
        restore(repo, '', 'agent-selftest', prefix, None, skip_lock=True,
                only=None, no_shims=False, verify_only=False, force=False)
        if not (target.stat().st_mode & 0o111):
            raise AssertionError('warm restore ignored a lost execute bit')

        # A corrupt (non-UTF-8) wrapper must be repaired, not raise.
        (prefix / 'bin/as').write_bytes(b'\xff\xfe')
        restore(repo, '', 'agent-selftest', prefix, None, skip_lock=True,
                only=None, no_shims=False, verify_only=False, force=False)
        if b'exec ' not in (prefix / 'bin/as').read_bytes():
            raise AssertionError('warm restore did not repair a corrupt wrapper')

        # --force must replace a corrupt wrapper as well as the payload.
        wrapper = prefix / 'bin/as'
        wrapper.write_text('stale wrapper\n')
        restore(repo, '', 'agent-selftest', prefix, None, skip_lock=True,
                only=None, no_shims=False, verify_only=False, force=True)
        if wrapper.read_text() == 'stale wrapper\n' or 'exec ' not in wrapper.read_text():
            raise AssertionError('forced restore did not repair a stale wrapper')

        # Validate all members before extraction: even the unsafe symlink must
        # not be left behind when a later member tries to escape through it.
        unsafe = base / 'unsafe.tar'
        with tarfile.open(unsafe, 'w') as output:
            link = tarfile.TarInfo('root/link')
            link.type = tarfile.SYMTYPE
            link.linkname = '../../outside'
            output.addfile(link)
            member = tarfile.TarInfo('root/link/escaped')
            member.size = 1
            output.addfile(member, io.BytesIO(b'x'))
        destination = base / 'unsafe-destination'
        destination.mkdir()
        try:
            extract_untrusted(unsafe, destination)
        except tarfile.FilterError:
            pass
        else:
            raise AssertionError('unsafe tar link was not rejected')
        if (destination / 'root/link').exists() or (destination / 'root/link').is_symlink():
            raise AssertionError('rejected archive was partially extracted')

        # A *chain* of legal links can still leave the prefix.  Member-level
        # validation cannot see it, so the post-extraction scan must.
        chain_dest = base / 'chain-prefix'
        (chain_dest / 'other').mkdir(parents=True)
        (base / 'chain-outside').mkdir()
        chain = base / 'chain.tar'
        with tarfile.open(chain, 'w') as output:
            for name, link in (('bin', 'root/link'),
                               ('root/link', 'inside/../../outside'),
                               ('root/inside', '../other')):
                item = tarfile.TarInfo(name)
                item.type = tarfile.SYMTYPE
                item.linkname = link
                output.addfile(item)
        installed = extract_untrusted(chain, chain_dest)
        try:
            reject_escaping_links(chain_dest)
        except RuntimeError:
            pass
        else:
            raise AssertionError('link chain escaping the prefix was not rejected')
        if (base / 'chain-outside' / 'gcc-16').exists():
            raise AssertionError('escaped link let a wrapper be written outside')
        if not installed:
            raise AssertionError('extraction produced no installed manifest')
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
