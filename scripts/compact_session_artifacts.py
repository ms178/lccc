#!/usr/bin/env python3
"""Losslessly compact a completed session snapshot; never prune checkpoint history.

Run only after lccc-snapshot.sh finishes. Archive receipts authenticate every old
member, including review tar streams. Repeated runs are safe; a same-name,
different-content checkpoint is an error. Only the rolling bundle may change:
its previous bytes are retained under a content-addressed member name.

Large temporary outputs live in --scratch (normally excluded .cache). A new
archive is installed before its receipt is atomically changed, then the old
archive is removed. Source conversion preserves the exact uncompressed tar.
The shared snapshot lock excludes a concurrent publisher. This is compaction,
not acceptance: run lccc_recover.py --offline --require-exact-history afterwards.
"""
from __future__ import annotations

import argparse
import fcntl
import gzip
import hashlib
import json
import lzma
import os
from pathlib import Path, PurePosixPath
import shutil
import sys
import tarfile
import tempfile


# The recovery helper is also a standalone deliverable, so keep the shared
# extraction budget there rather than giving it a new package dependency.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from lccc_recover import bundle_archive_within_budget


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def atomic_json(path, data):
    fd, name = tempfile.mkstemp(prefix='.receipt-', dir=path.parent)
    try:
        with os.fdopen(fd, 'w') as out:
            json.dump(data, out, indent=2)
            out.write('\n')
            out.flush()
            os.fsync(out.fileno())
        os.replace(name, path)
    finally:
        Path(name).unlink(missing_ok=True)


def safe_name(name):
    p = PurePosixPath(name)
    if not name or name == '.' or p.is_absolute() or '..' in p.parts or str(p) != name:
        raise ValueError('unsafe archive member: ' + name)
    return name


def decoded(path):
    return gzip.open(path, 'rb') if path.name.endswith('.gz') else path.open('rb')


class HashedReader:
    def __init__(self, stream):
        self.stream, self.hash, self.size = stream, hashlib.sha256(), 0

    def read(self, size=-1):
        data = self.stream.read(size)
        self.hash.update(data)
        self.size += len(data)
        return data



def publish_bundle_location(root, inventory):
    """The standalone recovery consumer reads SESSION_BUNDLE, not the inventory."""
    path = root / 'SESSION_BUNDLE.json'
    session = json.loads(path.read_text())
    archive = root / safe_name(inventory['archive'])
    if digest(archive) != inventory['archive_sha256']:
        raise ValueError('bundle archive checksum mismatch')
    matches = [row for row in inventory['members'] if row['name'] == session['bundle']]
    if len(matches) != 1 or matches[0]['sha256'] != session['bundle_sha256']:
        raise ValueError('current session bundle is not authenticated by inventory')
    session.update(bundle_archive=archive.name,
                   bundle_archive_sha256=inventory['archive_sha256'])
    atomic_json(path, session)
    return session


def compact_archive(root, receipt_name, additions, scratch, *, preset=9):
    """additions maps uncompressed member names to plain or gzip input paths."""
    receipt_path = root / receipt_name
    old = json.loads(receipt_path.read_text())
    old_path = root / safe_name(old['archive'])
    if digest(old_path) != old['archive_sha256']:
        raise ValueError('old archive checksum mismatch')
    expected = {safe_name(row['name']): row for row in old['members']}
    if len(expected) != len(old['members']):
        raise ValueError('duplicate old member')
    additions = dict(additions)
    incoming = {}
    for name, path in additions.items():
        safe_name(name)
        with decoded(path) as stream:
            reader = HashedReader(stream)
            while reader.read(1 << 20): pass
        incoming[name] = {'name': name, 'bytes': reader.size,
                          'sha256': reader.hash.hexdigest()}
    members, seen = [], set()
    with tempfile.TemporaryDirectory(dir=scratch) as td:
        output = Path(td) / 'archive.tar.xz'
        with lzma.open(output, 'wb', preset=preset) as compressed:
            with tarfile.open(fileobj=compressed, mode='w|') as archive:
                with tarfile.open(old_path, 'r|xz') as source:
                    for info in source:
                        name = safe_name(info.name)
                        if not info.isfile() or name in seen or name not in expected:
                            raise ValueError('unexpected/duplicate/non-file old member')
                        seen.add(name)
                        previous = expected[name]
                        out_name = name
                        if name in incoming and incoming[name]['sha256'] != previous['sha256']:
                            if name != 'lccc-session.bundle':
                                raise ValueError('conflicting checkpoint: ' + name)
                            out_name = 'lccc-session.' + previous['sha256'] + '.bundle'
                            if out_name in expected or out_name in incoming:
                                raise ValueError('renamed bundle collision')
                        elif name in incoming:
                            if incoming[name]['bytes'] != previous['bytes']:
                                raise ValueError('member size mismatch')
                            additions.pop(name)
                        reader = HashedReader(source.extractfile(info))
                        info.name = out_name
                        archive.addfile(info, reader)
                        if reader.size != previous['bytes'] or reader.hash.hexdigest() != previous['sha256']:
                            raise ValueError('old member checksum mismatch: ' + name)
                        members.append(dict(previous, name=out_name))
                if seen != set(expected):
                    raise ValueError('missing old members')
                for name, path in sorted(additions.items()):
                    row = incoming[name]
                    info = tarfile.TarInfo(name)
                    info.size, info.mode = row['bytes'], 0o644
                    with decoded(path) as stream:
                        reader = HashedReader(stream)
                        archive.addfile(info, reader)
                        if reader.size != row['bytes'] or reader.hash.hexdigest() != row['sha256'] or stream.read(1):
                            raise ValueError('input changed during compaction')
                    members.append(row)
        # Independently decompress and check the produced archive before publish.
        verify = {row['name']: row for row in members}
        if len(verify) != len(members):
            raise ValueError('duplicate output member')
        with tarfile.open(output, 'r|xz') as archive:
            for info in archive:
                row = verify.pop(info.name)
                reader = HashedReader(archive.extractfile(info))
                while reader.read(1 << 20): pass
                if reader.size != row['bytes'] or reader.hash.hexdigest() != row['sha256']:
                    raise ValueError('output verification failed')
        if verify:
            raise ValueError('output lost a member')
        if receipt_name == 'BUNDLE_ARCHIVE.json':
            total = 0
            for count, row in enumerate(members, 1):
                total += row['bytes']
                if not bundle_archive_within_budget(count, row['bytes'], total):
                    raise ValueError('bundle archive exceeds recovery budget; old publication retained')
        sha = digest(output)
        destination = root / (receipt_name.removesuffix('.json').lower() + '.' + sha[:16] + '.tar.xz')
        os.replace(output, destination)
        result = dict(old, archive=destination.name, archive_sha256=sha, members=members)
        atomic_json(receipt_path, result)
        # Keep the old archive until BOTH pointers have been updated. The
        # recovery consumer otherwise loses its bundle when the loose input
        # is removed, even though the inventory itself is perfectly valid.
        if receipt_name == 'BUNDLE_ARCHIVE.json':
            publish_bundle_location(root, result)
        if old_path != destination:
            old_path.unlink()
    # Caller owns input lifetime; no unchecked recursive cleanup.
    return result


def compact_source(root, scratch, *, preset=9):
    receipt_path = root / 'SOURCE_ARCHIVE.json'
    data = json.loads(receipt_path.read_text())
    source = root / safe_name(data['archive'])
    if digest(source) != data['archive_sha256']:
        raise ValueError('source archive checksum mismatch')
    if source.suffix == '.xz':
        return data
    if not source.name.endswith('.tar.gz'):
        raise ValueError('expected gzip source tar')
    with tempfile.TemporaryDirectory(dir=scratch) as td:
        output = Path(td) / 'source.tar.xz'
        with gzip.open(source, 'rb') as raw, lzma.open(output, 'wb', preset=preset) as out:
            reader = HashedReader(raw)
            shutil.copyfileobj(reader, out, 1 << 20)
        with lzma.open(output, 'rb') as raw:
            check = hashlib.file_digest(raw, 'sha256').hexdigest()
        if check != reader.hash.hexdigest():
            raise ValueError('source tar stream changed')
        sha = digest(output)
        destination = root / ('lccc-src.' + sha[:16] + '.tar.xz')
        os.replace(output, destination)
        data.update(archive=destination.name, archive_sha256=sha,
                    uncompressed_tar_sha256=check)
        atomic_json(receipt_path, data)
        source.unlink()
    return data


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--artifacts', type=Path, required=True)
    parser.add_argument('--scratch', type=Path, required=True)
    parser.add_argument('--series-tag', required=True, help='e.g. S40; immutable review archive identity')
    parser.add_argument('--checkpoint', action='append', default=[], metavar='MEMBER=PATH')
    args = parser.parse_args()
    root, scratch = args.artifacts.resolve(), args.scratch.resolve()
    scratch.mkdir(parents=True, exist_ok=True)
    with (root / '.snapshot.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        additions = {p.name.removesuffix('.gz'): p for p in root.glob('ms178-1.S*.patch.gz')}
        cleanup = set(additions.values())
        for entry in args.checkpoint:
            name, path = entry.split('=', 1)
            if name in additions: raise ValueError('duplicate additional checkpoint')
            additions[name] = Path(path)
        series = root / 'series.tar.gz'
        if series.exists():
            additions['review-series.' + args.series_tag + '.tar'] = series
            cleanup.add(series)
        history = compact_archive(root, 'RECOVERY_HISTORY_ARCHIVE.json', additions, scratch)
        for path in cleanup: path.unlink()
        bundle = root / 'lccc-session.bundle'
        bundles = compact_archive(root, 'BUNDLE_ARCHIVE.json',
                                  {'lccc-session.bundle': bundle} if bundle.exists() else {}, scratch)
        if bundle.exists(): bundle.unlink()
        source = compact_source(root, scratch)
        print(json.dumps({'history_members': len(history['members']),
                          'bundle_members': len(bundles['members']),
                          'source_archive': source['archive']}, indent=2))


if __name__ == '__main__':
    main()
