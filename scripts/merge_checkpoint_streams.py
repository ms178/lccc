#!/usr/bin/env python3
"""Losslessly merge XZ-compressed TAR byte streams with a shared dictionary.

Keep all original TAR headers, members and end markers, including duplicate
manifest names. This is a concatenated TAR: use tar --ignore-zeros to traverse
all generations. Verify each decompressed segment before removing any source.
"""
import argparse
import fcntl
import hashlib
import json
import lzma
import os
import tempfile
from pathlib import Path

BLOCK = 1 << 20


def sha(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(BLOCK), b''):
            h.update(block)
    return h.hexdigest()


def merge(sources, destination, remove=False):
    sources = [Path(p) for p in sources]
    destination = Path(destination)
    if (not sources or len(set(p.resolve() for p in sources)) != len(sources)
            or destination.exists() or destination.is_symlink()
            or any(p.is_symlink() or not p.is_file() or p.resolve() == destination.resolve()
                   for p in sources)):
        raise ValueError('require distinct regular sources and a new destination')
    receipt = {'archive': str(destination), 'format': 'concatenated TAR byte streams', 'members': {}}
    with (destination.parent / '.snapshot.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if destination.exists() or destination.is_symlink():
            raise ValueError('destination appeared while waiting for publication lock')
        fd, temporary_name = tempfile.mkstemp(prefix='.' + destination.name + '.', dir=destination.parent)
        os.close(fd)
        temporary = Path(temporary_name)
        try:
            with lzma.open(temporary, 'wb', filters=[dict(id=lzma.FILTER_LZMA2,
                                                        dict_size=64 << 20, preset=4)]) as out:
                for i, source in enumerate(sources):
                    row = {'source': str(source), 'source_sha256': sha(source), 'tar_bytes': 0}
                    h = hashlib.sha256()
                    with lzma.open(source, 'rb') as inp:
                        for block in iter(lambda: inp.read(BLOCK), b''):
                            h.update(block)
                            row['tar_bytes'] += len(block)
                            out.write(block)
                    row['tar_sha256'] = h.hexdigest()
                    receipt['members'][str(i)] = row
            with lzma.open(temporary, 'rb') as stream:
                for row in receipt['members'].values():
                    remaining = row['tar_bytes']
                    h = hashlib.sha256()
                    while remaining:
                        block = stream.read(min(BLOCK, remaining))
                        if not block:
                            raise ValueError('truncated merged archive')
                        h.update(block)
                        remaining -= len(block)
                    if h.hexdigest() != row['tar_sha256']:
                        raise ValueError('archive segment verification failed')
                if stream.read(1):
                    raise ValueError('unexpected archive suffix')
            for row in receipt['members'].values():
                if sha(Path(row['source'])) != row['source_sha256']:
                    raise ValueError('source changed during compression')
            receipt['sha256'] = sha(temporary)
            receipt['bytes'] = temporary.stat().st_size
            with temporary.open('rb') as stream:
                os.fsync(stream.fileno())
            os.replace(temporary, destination)
            receipt_path = Path(str(destination) + '.receipt.json')
            fd, receipt_name = tempfile.mkstemp(prefix='.' + receipt_path.name + '.', dir=destination.parent)
            receipt_tmp = Path(receipt_name)
            with os.fdopen(fd, 'w') as stream:
                json.dump(receipt, stream, indent=2)
                stream.write('\n')
                stream.flush()
                os.fsync(stream.fileno())
            os.replace(receipt_tmp, receipt_path)
            if remove:
                for source in sources:
                    source.unlink()
            return receipt
        finally:
            temporary.unlink(missing_ok=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--output', type=Path, required=True)
    ap.add_argument('--remove-after-verify', action='store_true')
    ap.add_argument('sources', nargs='+', type=Path)
    args = ap.parse_args()
    print(json.dumps(merge(args.sources, args.output, args.remove_after_verify), indent=2))


if __name__ == '__main__':
    main()
