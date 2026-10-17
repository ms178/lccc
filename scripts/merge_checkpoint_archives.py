#!/usr/bin/env python3
"""Losslessly merge XZ checkpoint archives using a shared compression dictionary.

Each original decompressed TAR stream is preserved byte-for-byte as one member.
Nothing is removed before every member has been read back and SHA256 verified.
Recover an old TAR with `tar -xJOf COMBINED OLD-NAME.tar > OLD-NAME.tar`.
"""
import argparse
import fcntl
import hashlib
import io
import json
import lzma
import os
from pathlib import Path
import tarfile


def scan(path, opener=open):
    h = hashlib.sha256()
    size = 0
    with opener(path, 'rb') as stream:
        for chunk in iter(lambda: stream.read(1 << 20), b''):
            size += len(chunk)
            h.update(chunk)
    return h.hexdigest(), size


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--remove-after-verify', action='store_true')
    p.add_argument('sources', type=Path, nargs='+')
    args = p.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)
    tmp = Path(str(args.out) + '.tmp')
    with (args.out.parent / '.snapshot.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if args.out.exists() or len({x.name for x in args.sources}) != len(args.sources):
            raise ValueError('existing destination or duplicate source name')
        rows = {}
        for path in args.sources:
            if not path.name.endswith('.tar.xz'):
                raise ValueError('expected an XZ TAR archive')
            raw_hash, raw_size = scan(path, lzma.open)
            rows[path.name[:-3]] = dict(source=str(path), source_sha256=scan(path)[0],
                                       tar_sha256=raw_hash, tar_bytes=raw_size)
        with lzma.open(tmp, 'wb', filters=[dict(id=lzma.FILTER_LZMA2,
                                              dict_size=64 << 20, preset=4)]) as stream:
            with tarfile.open(fileobj=stream, mode='w|') as archive:
                for name, row in rows.items():
                    info = tarfile.TarInfo(name)
                    info.size = row['tar_bytes']
                    with lzma.open(row['source'], 'rb') as inp:
                        archive.addfile(info, inp)
                payload = (json.dumps(rows, indent=2) + '\n').encode()
                info = tarfile.TarInfo('MANIFEST.json')
                info.size = len(payload)
                archive.addfile(info, io.BytesIO(payload))
        with tarfile.open(tmp, 'r|xz') as archive:
            seen = set()
            for member in archive:
                if member.name == 'MANIFEST.json':
                    if json.load(archive.extractfile(member)) != rows:
                        raise ValueError('manifest changed')
                    continue
                row = rows[member.name]
                h = hashlib.sha256()
                stream = archive.extractfile(member)
                for chunk in iter(lambda: stream.read(1 << 20), b''):
                    h.update(chunk)
                if member.size != row['tar_bytes'] or h.hexdigest() != row['tar_sha256']:
                    raise ValueError('TAR round-trip failed: ' + member.name)
                seen.add(member.name)
            if seen != set(rows):
                raise ValueError('missing member')
        os.replace(tmp, args.out)
        receipt = dict(archive=str(args.out), sha256=scan(args.out)[0], members=rows)
        receipt_path = Path(str(args.out) + '.receipt.json')
        receipt_tmp = Path(str(receipt_path) + '.tmp')
        receipt_tmp.write_text(json.dumps(receipt, indent=2) + '\n')
        os.replace(receipt_tmp, receipt_path)
        if args.remove_after_verify:
            for row in rows.values():
                path = Path(row['source'])
                if scan(path)[0] != row['source_sha256']:
                    raise ValueError('source changed: ' + str(path))
                path.unlink()
        print(f"VERIFIED {len(rows)} byte-identical TAR streams in {args.out}: "
              f"{args.out.stat().st_size} bytes")


if __name__ == '__main__':
    main()
