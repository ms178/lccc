#!/usr/bin/env python3
"""Losslessly repack named patch checkpoints; never discard patch history.

Older whole-patch checkpoints share most of their plaintext, but individual
GZIP streams cannot share a dictionary. A verified XZ tar keeps every named
patch and SHA256 with a shared dictionary. Originals are removed ONLY after
atomic publication and rereading every archive member. The canonical patch,
recovery bundle, source archive and checkpoint ledger are never modified.
"""
import argparse
import fcntl
import gzip
import hashlib
import io
import json
import lzma
import os
from pathlib import Path
import re
import tarfile


def digest(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--artifacts', type=Path, required=True)
    parser.add_argument('--through', type=int, required=True)
    parser.add_argument('--remove-after-verify', action='store_true')
    args = parser.parse_args()
    dest = args.artifacts / f'checkpoint-history-through-S{args.through}.tar.xz'
    temporary = dest.with_suffix('.tmp')
    with (args.artifacts / '.snapshot.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        files = []
        for path in sorted(args.artifacts.glob('ms178-1.S*.patch.gz')):
            match = re.search(r'\.S(\d+)-', path.name)
            if match and int(match.group(1)) <= args.through:
                files.append(path)
        if not files or dest.exists():
            raise SystemExit('no matching checkpoints, or archive already exists; refusing overwrite')
        manifest = {}
        with lzma.open(temporary, 'wb', filters=[dict(id=lzma.FILTER_LZMA2, dict_size=64 << 20, preset=4)]) as stream:
            with tarfile.open(fileobj=stream, mode='w|') as archive:
                for path in files:
                    packed = path.read_bytes()
                    data = gzip.decompress(packed)
                    name = path.name[:-3]
                    manifest[name] = dict(patch_sha256=digest(data), patch_bytes=len(data), original_gzip=path.name, gzip_sha256=digest(packed))
                    info = tarfile.TarInfo(name)
                    info.size = len(data)
                    info.mode = 0o644
                    archive.addfile(info, io.BytesIO(data))
                data = (json.dumps(manifest, indent=2) + '\n').encode()
                info = tarfile.TarInfo('MANIFEST.json')
                info.size = len(data)
                info.mode = 0o644
                archive.addfile(info, io.BytesIO(data))
        with tarfile.open(temporary, 'r:xz') as archive:
            if set(archive.getnames()) != set(manifest) | {'MANIFEST.json'}:
                raise RuntimeError('archive member mismatch')
            for name, row in manifest.items():
                data = archive.extractfile(name).read()
                if len(data) != row['patch_bytes'] or digest(data) != row['patch_sha256']:
                    raise RuntimeError('archive round-trip failed: ' + name)
        os.replace(temporary, dest)
        if args.remove_after_verify:
            for path in files:
                # Fail closed if anything changed despite the publisher lock.
                if digest(path.read_bytes()) != manifest[path.name[:-3]]['gzip_sha256']:
                    raise RuntimeError('checkpoint changed: ' + str(path))
                path.unlink()
        print(f'VERIFIED {len(files)} named patches in {dest} ({dest.stat().st_size} bytes)')
        print('Restore any named plaintext patch with: tar -xJOf ARCHIVE NAME.patch > restored.patch')


if __name__ == '__main__':
    main()
