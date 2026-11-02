#!/usr/bin/env python3
"""Losslessly repack raw evidence tarballs with verification before removal.

The decompressed TAR byte stream is preserved exactly, including every raw
counter file and its metadata. This is storage deduplication, not filtering.
"""
import argparse
import gzip
import hashlib
import json
import lzma
import os
from pathlib import Path


def digest(path, opener=open):
    h = hashlib.sha256()
    with opener(path, 'rb') as stream:
        for chunk in iter(lambda: stream.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('--receipt', type=Path, required=True)
    args = parser.parse_args()
    rows = []
    for source in sorted(args.directory.rglob('*-raw.tar.gz')):
        target = Path(str(source)[:-3] + '.xz')
        temporary = Path(str(target) + '.tmp')
        if target.exists():
            raise RuntimeError('refusing overwrite: ' + str(target))
        original = digest(source)
        plain = hashlib.sha256()
        with gzip.open(source, 'rb') as inp, lzma.open(temporary, 'wb', preset=4) as out:
            for chunk in iter(lambda: inp.read(1 << 20), b''):
                plain.update(chunk)
                out.write(chunk)
        if digest(temporary, lzma.open) != plain.hexdigest() or digest(source) != original:
            raise RuntimeError('verification failed: ' + str(source))
        os.replace(temporary, target)
        rows.append(dict(source=str(source), target=str(target), gzip_sha256=original,
                         tar_sha256=plain.hexdigest(), xz_sha256=digest(target),
                         before_bytes=source.stat().st_size, after_bytes=target.stat().st_size))
        # Publish a durable receipt for each verified pair before deletion.
        receipt_tmp = Path(str(args.receipt) + '.tmp')
        receipt_tmp.write_text(json.dumps(rows, indent=2) + '\n')
        os.replace(receipt_tmp, args.receipt)
        source.unlink()
    print(f'{len(rows)} archives verified; {sum(r["before_bytes"] for r in rows)} -> '
          f'{sum(r["after_bytes"] for r in rows)} bytes; no raw evidence discarded')


if __name__ == '__main__':
    main()
