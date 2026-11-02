#!/usr/bin/env python3
"""Archive PR829/830 experiment files, verifying every member before publication.

Compiler executables named synthesis-v* are omitted (their hashes are retained;
source/history and build recipes are delivered). Benchmark binaries, failed
experiments, raw profiles, commands and outputs are not filtered out.
"""
import argparse
import hashlib
import io
import json
import lzma
import os
from pathlib import Path
import tarfile


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--cache', type=Path, required=True)
    p.add_argument('--logs', type=Path, required=True)
    p.add_argument('--out', type=Path, required=True)
    args = p.parse_args()
    if args.out.exists():
        raise ValueError('refusing to overwrite an evidence archive')
    rows, excluded, paths = {}, {}, {}
    for prefix, root in [('cache', args.cache), ('logs', args.logs)]:
        for path in sorted(root.rglob('*')):
            if not path.is_file() or path.is_symlink():
                continue
            name = prefix + '/' + path.relative_to(root).as_posix()
            record = dict(bytes=path.stat().st_size, sha256=digest(path))
            if prefix == 'cache' and path.parent == root and path.name.startswith('synthesis-v'):
                excluded[name] = record
            else:
                rows[name], paths[name] = record, path
    tmp = Path(str(args.out) + '.tmp')
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with lzma.open(tmp, 'wb', filters=[dict(id=lzma.FILTER_LZMA2, dict_size=64 << 20, preset=4)]) as stream:
        with tarfile.open(fileobj=stream, mode='w|') as archive:
            for name, path in paths.items():
                archive.add(path, arcname=name, recursive=False)
            data = (json.dumps(dict(files=rows, excluded_compilers=excluded), indent=2) + '\n').encode()
            info = tarfile.TarInfo('MANIFEST.json')
            info.size = len(data)
            archive.addfile(info, io.BytesIO(data))
    seen = set()
    with tarfile.open(tmp, 'r|xz') as archive:
        for member in archive:
            if member.name == 'MANIFEST.json':
                if json.load(archive.extractfile(member)) != dict(files=rows, excluded_compilers=excluded):
                    raise ValueError('manifest mismatch')
                continue
            h = hashlib.sha256()
            stream = archive.extractfile(member)
            for chunk in iter(lambda: stream.read(1 << 20), b''):
                h.update(chunk)
            expected = rows[member.name]
            if member.size != expected['bytes'] or h.hexdigest() != expected['sha256']:
                raise ValueError('changed or corrupted evidence: ' + member.name)
            seen.add(member.name)
        if seen != set(rows):
            raise ValueError('missing archive members')
    os.replace(tmp, args.out)
    receipt = dict(archive=str(args.out), sha256=digest(args.out), bytes=args.out.stat().st_size,
                   verified_members=len(rows), raw_bytes=sum(r['bytes'] for r in rows.values()),
                   excluded_compilers=excluded)
    receipt_path = Path(str(args.out) + '.receipt.json')
    receipt_tmp = Path(str(receipt_path) + '.tmp')
    receipt_tmp.write_text(json.dumps(receipt, indent=2) + '\n')
    os.replace(receipt_tmp, receipt_path)
    print(json.dumps(receipt, indent=2))


if __name__ == '__main__':
    main()
