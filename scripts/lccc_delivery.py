#!/usr/bin/env python3
"""Publish a small, self-contained download of the canonical session patch.

The .patch itself remains in the visible workspace. A ZIP (not another large
text preview) contains its exact bytes, SHA-256, base/head and honest CI status.
All writes are same-directory fsync/rename. The ZIP is the commit point; sidecar
receipts are independently atomic, not a fictional multi-file transaction.

Workspace budgets count *all* regular files, including otherwise-excluded
caches. Keep checkouts/toolchains outside this small delivery workspace. Never
delete user files to meet a budget: fail loudly and preserve the previous ZIP.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import tempfile
import zipfile

MIB = 1024 * 1024
MAX_WORKSPACE_BYTES = 64 * MIB  # headroom below the approximate 128 MiB cap
MAX_WORKSPACE_FILES = 256      # headroom below the approximate 10,000-file cap
SHA = re.compile(r'^[0-9a-f]{40}$')
BLOCK = 1024 * 1024


class DeliveryError(RuntimeError):
    """Publication failed; this is never reported as a successful delivery."""


def digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda: f.read(BLOCK), b''):
            h.update(block)
    return h.hexdigest()


def inventory(root: Path, *, exclude: set[Path] = frozenset()) -> dict:
    """Bounded, non-symlink-following census; excluded items are owned temps."""
    files = size = symlinks = 0
    def fail(exc):
        raise DeliveryError(f'cannot inventory workspace: {exc}') from exc
    for directory, dirs, names in os.walk(root, followlinks=False, onerror=fail):
        for name in list(dirs):
            p = Path(directory) / name
            if p.is_symlink():
                symlinks += 1; files += 1; size += p.lstat().st_size
                dirs.remove(name)
        for name in names:
            p = Path(directory) / name
            if p in exclude:
                continue
            s = p.lstat()
            if stat.S_ISREG(s.st_mode) or stat.S_ISLNK(s.st_mode):
                files += 1; size += s.st_size
                symlinks += stat.S_ISLNK(s.st_mode)
            else:
                raise DeliveryError(f'non-regular workspace entry: {p}')
    return dict(files=files, bytes=size, symlinks=symlinks)


def check_budget(current: dict, outputs: dict[Path, int], *, max_bytes: int,
                 max_files: int) -> dict:
    if type(max_bytes) is not int or max_bytes <= 0 or type(max_files) is not int or max_files <= 0:
        raise DeliveryError('workspace budgets must be positive integers')
    projected = dict(current)
    for path, size in outputs.items():
        if path.is_symlink():
            raise DeliveryError(f'refusing to replace symlink output: {path}')
        if path.exists():
            if not path.is_file():
                raise DeliveryError(f'output is not a regular file: {path}')
            projected['bytes'] -= path.stat().st_size
        else:
            projected['files'] += 1
        projected['bytes'] += size
    if projected['bytes'] > max_bytes or projected['files'] > max_files:
        raise DeliveryError(f'workspace budget exceeded: {projected}; '
                            f'limits bytes={max_bytes}, files={max_files}. '
                            'Move bulk work outside the workspace; no user files were deleted.')
    return projected


@contextmanager
def writer_lock(root: Path):
    # POSIX is the snapshot script's execution contract. Importing this module
    # remains side-effect free; even flock discovery occurs only on publication.
    import fcntl
    lock = root / '.lccc-delivery.lock'
    if lock.is_symlink():
        raise DeliveryError('delivery lock must not be a symlink')
    with lock.open('a+b') as f:
        try:
            fcntl.flock(f.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as exc:
            raise DeliveryError('another delivery publisher is active') from exc
        try:
            yield
        finally:
            fcntl.flock(f.fileno(), fcntl.LOCK_UN)


def sync_directory(path: Path) -> None:
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def stage_bytes(path: Path, data: bytes) -> Path:
    fd, name = tempfile.mkstemp(prefix='.' + path.name + '-', suffix='.tmp', dir=path.parent)
    staged = Path(name)
    try:
        with os.fdopen(fd, 'wb') as f:
            f.write(data); f.flush(); os.fsync(f.fileno())
        staged.chmod(0o644)
        return staged
    except BaseException:
        staged.unlink(missing_ok=True)
        raise


def member(name: str) -> zipfile.ZipInfo:
    # Deterministic metadata, independent of source file timestamps/umask.
    info = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
    info.compress_type = zipfile.ZIP_DEFLATED
    info.external_attr = 0o100644 << 16
    info.create_system = 3
    return info


def canonical_json(value: dict) -> bytes:
    return (json.dumps(value, indent=2, sort_keys=True, ensure_ascii=True,
                       allow_nan=False) + '\n').encode('utf-8')


def publish(patch: Path, workspace: Path, metadata: dict, *,
            max_bytes: int = MAX_WORKSPACE_BYTES,
            max_files: int = MAX_WORKSPACE_FILES) -> dict:
    workspace = workspace.resolve(strict=True)
    # Reject the original path's symlink before resolving it.
    if patch.is_symlink() or not patch.is_file():
        raise DeliveryError('canonical patch must be a regular, non-symlink file')
    patch = patch.resolve(strict=True)
    if patch.parent != workspace or patch.name != 'ms178-1.patch':
        raise DeliveryError('canonical ms178-1.patch must be directly in the delivery workspace')
    if not patch.stat().st_size:
        raise DeliveryError('refusing to publish an empty patch')
    required = {'base', 'head', 'snapshot', 'ci_gate'}
    if set(metadata) != required or any(not isinstance(v, str) or not v for v in metadata.values()):
        raise DeliveryError('base/head/snapshot/ci_gate metadata required, without unknown keys')
    if not SHA.fullmatch(metadata['base']) or not SHA.fullmatch(metadata['head']):
        raise DeliveryError('base and head must be full Git commit IDs')
    if not re.fullmatch(r'S[0-9]+-[A-Za-z0-9._-]+', metadata['snapshot']):
        raise DeliveryError('invalid snapshot tag')
    destinations = [workspace/'ms178-1.patch.zip', workspace/'LCCC-DELIVERY.json', workspace/'LCCC-DELIVERY.md']
    for path in destinations:
        if path.is_symlink() or (path.exists() and not path.is_file()):
            raise DeliveryError(f'unsafe output: {path}')
    temps = set()
    with writer_lock(workspace):
        # Reject already oversized workspaces before creating another archive.
        check_budget(inventory(workspace), {}, max_bytes=max_bytes, max_files=max_files)
        identity = patch.stat()
        patch_sha = digest(patch)
        manifest = dict(format='lccc-patch-delivery-v1', **metadata,
                        patch='ms178-1.patch', patch_bytes=identity.st_size, patch_sha256=patch_sha)
        readme = (f'# LCCC – Patch-Lieferung\n\n'
                  f'**Basis:** `{metadata["base"]}`  \n'
                  f'**Head:** `{metadata["head"]}`  \n'
                  f'**Validierungsstatus:** `{metadata["ci_gate"]}`\n\n'
                  'Die ZIP enthält die vollständige `ms178-1.patch`, nicht nur einen Link. '
                  'ZIP und Patch sind bewusst getrennt vom großen Compiler-Arbeitsbaum.\n\n'
                  '```sh\n# Im Repository auf dem oben genannten Basis-Commit:\n'
                  'git apply --check ms178-1.patch\ngit apply ms178-1.patch\n```\n\n'
                  f'Patch: **{identity.st_size:,} Bytes**  \nSHA-256: `{patch_sha}`\n\n'
                  '`UNGATED` bedeutet Zwischenstand, **keine** bestandene Gesamt-CI. '
                  'Der Stand wurde lokal mit `ci_local.sh --fast` geprüft; GitHub CI führt die übrigen Gates aus. '
                  'Einzelne validierte Änderungen stehen im Snapshot-Ledger; '
                  'der eingebettete Status wird nicht durch die Transportprüfung aufgewertet.\n').encode('utf-8')
        fd, name = tempfile.mkstemp(prefix='.ms178-1.zip-', suffix='.tmp', dir=workspace)
        os.close(fd); archive = Path(name); temps.add(archive)
        try:
            with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_DEFLATED, compresslevel=9) as z:
                with patch.open('rb') as src, z.open(member('ms178-1.patch'), 'w', force_zip64=True) as dst:
                    for block in iter(lambda: src.read(BLOCK), b''):
                        dst.write(block)
                z.writestr(member('DELIVERY.json'), canonical_json(manifest))
                z.writestr(member('README.md'), readme)
            # Verify CRC AND SHA-256 using a bounded streaming read, not a
            # second 20+ MiB in-memory copy of the entire patch.
            with zipfile.ZipFile(archive) as z:
                if z.testzip() is not None:
                    raise DeliveryError('ZIP CRC verification failed')
                h = hashlib.sha256()
                with z.open('ms178-1.patch') as src:
                    for block in iter(lambda: src.read(BLOCK), b''):
                        h.update(block)
                if h.hexdigest() != patch_sha:
                    raise DeliveryError('ZIP patch differs from canonical patch')
            now = patch.stat()
            if (now.st_dev, now.st_ino, now.st_size, now.st_mtime_ns) != (
                    identity.st_dev, identity.st_ino, identity.st_size, identity.st_mtime_ns) or digest(patch) != patch_sha:
                raise DeliveryError('canonical patch changed during publication')
            with archive.open('rb') as f:
                os.fsync(f.fileno())
            archive.chmod(0o644)
            receipt = dict(manifest, zip='ms178-1.patch.zip', zip_bytes=archive.stat().st_size,
                           zip_sha256=digest(archive))
            receipt_data = canonical_json(receipt)
            projected = check_budget(inventory(workspace, exclude=temps),
                                     {destinations[0]: archive.stat().st_size,
                                      destinations[1]: len(receipt_data), destinations[2]: len(readme)},
                                     max_bytes=max_bytes, max_files=max_files)
            receipt_tmp = stage_bytes(destinations[1], receipt_data); temps.add(receipt_tmp)
            readme_tmp = stage_bytes(destinations[2], readme); temps.add(readme_tmp)
            # The self-contained ZIP is the primary delivery commit point.
            for staged, destination in zip((archive, receipt_tmp, readme_tmp), destinations):
                os.replace(staged, destination); temps.discard(staged)
                sync_directory(workspace)
            return dict(receipt, workspace=projected)
        finally:
            for p in temps:
                p.unlink(missing_ok=True)


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--patch', type=Path, default=Path('/home/user/ms178-1.patch'))
    ap.add_argument('--workspace', type=Path, default=Path('/home/user'))
    ap.add_argument('--base', required=True)
    ap.add_argument('--head', required=True)
    ap.add_argument('--snapshot', required=True)
    ap.add_argument('--ci-gate', required=True)
    ap.add_argument('--budget-mib', type=int, default=64)
    ap.add_argument('--file-budget', type=int, default=256)
    args = ap.parse_args(argv)
    try:
        result = publish(args.patch, args.workspace,
                         dict(base=args.base, head=args.head, snapshot=args.snapshot, ci_gate=args.ci_gate),
                         max_bytes=args.budget_mib*MIB, max_files=args.file_budget)
    except (DeliveryError, OSError, ValueError, zipfile.BadZipFile) as exc:
        ap.exit(1, f'delivery FAILED: {exc}\n')
    print(json.dumps(result, indent=2, sort_keys=True, allow_nan=False))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
