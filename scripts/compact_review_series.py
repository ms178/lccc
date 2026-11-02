#!/usr/bin/env python3
"""Losslessly compact the disposable format-patch TAR after a snapshot.

Recovery uses the independent source archive and Git bundle. Preserve every
byte of the decompressed review TAR, verify it before atomic publication, and
remove the redundant gzip only after rechecking its identity. The snapshot
publisher may regenerate series.tar.gz; rerun this helper after publication.
"""
import argparse
import hashlib
import gzip
import lzma
import json
import os
from pathlib import Path


def digest(path, opener=open):
    h = hashlib.sha256()
    with opener(path, 'rb') as stream:
        for block in iter(lambda: stream.read(1 << 20), b''):
            h.update(block)
    return h.hexdigest()


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('source', type=Path)
    args = ap.parse_args()
    source = args.source
    if not source.name.endswith('.tar.gz') or source.is_symlink():
        ap.error('expected a regular .tar.gz archive')
    dest = source.with_suffix('.xz')
    original = digest(source)
    plain = digest(source, gzip.open)
    temporary = Path(str(dest) + '.tmp')
    with gzip.open(source, 'rb') as inp, lzma.open(temporary, 'wb', preset=4) as out:
        for block in iter(lambda: inp.read(1 << 20), b''):
            out.write(block)
    if digest(temporary, lzma.open) != plain or digest(source) != original:
        raise ValueError('archive changed or failed its round trip')
    os.replace(temporary, dest)
    receipt = dict(source=str(source), source_sha256=original, tar_sha256=plain,
                   archive=str(dest), archive_sha256=digest(dest), bytes=dest.stat().st_size)
    receipt_path = Path(str(dest) + '.receipt.json')
    receipt_tmp = Path(str(receipt_path) + '.tmp')
    receipt_tmp.write_text(json.dumps(receipt, indent=2) + '\n')
    os.replace(receipt_tmp, receipt_path)
    source.unlink()
    print(json.dumps(receipt, indent=2))


if __name__ == '__main__':
    main()
