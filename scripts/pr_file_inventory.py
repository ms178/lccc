#!/usr/bin/env python3
"""Reconcile a complete GitHub PR files response with Git/worktree identities.

Identity is NOT semantic review. In particular, zero API change counts are not
proof of identical content. Reads only; no network or repository mutations.
"""
import argparse
from collections import Counter
import json
from pathlib import Path, PurePosixPath
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def git(*args, optional=False):
    result = subprocess.run(['git', *args], cwd=ROOT, text=True, capture_output=True)
    if result.returncode and not optional:
        raise ValueError(result.stderr)
    return result.stdout.strip() if result.returncode == 0 else None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--files', type=Path, required=True)
    parser.add_argument('--expected-count', type=int, required=True)
    parser.add_argument('--donor', required=True)
    parser.add_argument('--main', required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    files = json.loads(args.files.read_text())
    if len(files) != args.expected_count or len({f['filename'] for f in files}) != len(files):
        parser.error('incomplete/duplicate PR inventory')
    donor = git('rev-parse', '--verify', '--end-of-options', args.donor + '^{commit}')
    base = git('rev-parse', '--verify', '--end-of-options', args.main + '^{commit}')
    rows = []
    for entry in files:
        name = entry['filename']
        path = PurePosixPath(name)
        if path.is_absolute() or '..' in path.parts or not (ROOT / name).resolve().is_relative_to(ROOT):
            parser.error('unsafe PR path')
        old = git('rev-parse', '--verify', f'{donor}:{name}', optional=True)
        upstream = git('rev-parse', '--verify', f'{base}:{name}', optional=True)
        work = git('hash-object', '--', name) if (ROOT / name).is_file() else None
        state = ('exact-donor' if work == old else 'upstream-main' if work == upstream
                 else 'synthesis-differs' if work else 'absent')
        rows.append(dict(path=name, state=state, donor_blob=old, main_blob=upstream, work_blob=work))
    report = dict(note='Content identity inventory, NOT semantic review proof. Local diffs override API change statistics.',
                  donor=donor, main=base, counts=dict(Counter(r['state'] for r in rows)), files=rows)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    temporary = args.out.with_suffix(args.out.suffix + '.tmp')
    temporary.write_text(json.dumps(report, indent=2) + '\n')
    temporary.replace(args.out)
    print(report['counts'])


if __name__ == '__main__':
    main()
