#!/usr/bin/env python3
"""Stage and atomically preserve the complete binary diff, including new files.

The receipt binds its exact base commit and both compressed/plain hashes.
This is an emergency recovery delta, never a validation or delivery stamp.
"""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def atomic(path, data):
    fd, temporary = tempfile.mkstemp(prefix='.' + path.name, dir=path.parent)
    try:
        with os.fdopen(fd, 'wb') as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    def git(*argv):
        return subprocess.check_output(['git', *argv], cwd=ROOT)
    git('add', '-A')
    base = git('rev-parse', 'HEAD').decode().strip()
    raw = git('diff', '--binary', '--no-color', 'HEAD')
    compressed = gzip.compress(raw, mtime=0)
    assert gzip.decompress(compressed) == raw
    args.out.mkdir(parents=True, exist_ok=True)
    digest = hashlib.sha256(raw).hexdigest()
    # Content-addressed filenames keep an interrupted later save from
    # overwriting the diff named by the previous complete receipt.
    path = args.out / f'inflight-{base[:12]}-{digest[:12]}.diff.gz'
    atomic(path, compressed)
    assert gzip.decompress(path.read_bytes()) == raw
    receipt = dict(base=base, file=path.name, raw_sha256=digest,
                   gzip_sha256=hashlib.sha256(compressed).hexdigest(), raw_bytes=len(raw),
                   status='UNVALIDATED emergency delta, apply only to the exact base')
    atomic(args.out / 'INFLIGHT.json', (json.dumps(receipt, indent=2) + '\n').encode())
    print(json.dumps(receipt))


if __name__ == '__main__':
    main()
