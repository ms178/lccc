#!/usr/bin/env python3
"""Replay a saved native corpus's exact commands; identify invalidated timings.

An unchanged executable can retain its old measurements. A changed executable
must be remeasured; this tool does not infer correctness or performance for it.
"""
import argparse
import hashlib
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from tools.corpus import process, publication


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument('--report', type=Path, required=True)
    p.add_argument('--hashes', type=Path, required=True,
                   help='original per-workload/arm binary_sha256 map, saved before rebuilding')
    p.add_argument('--compiler', type=Path, required=True)
    p.add_argument('--out', type=Path, required=True)
    args = p.parse_args(argv)
    original = json.loads(args.report.read_text())
    hashes = json.loads(args.hashes.read_text())['binary_sha256']
    args.out.mkdir(parents=True, exist_ok=True)
    result = dict(complete=False, original_report_sha256=digest(args.report),
                  original_hashes_sha256=digest(args.hashes),
                  compiler_sha256=digest(args.compiler), checks=[], changed=[], failures=[])
    def save():
        publication.atomic_bytes(args.out / 'replay.json',
                                 (json.dumps(result, indent=2) + '\n').encode())
    with publication.lock(args.out / '.replay.lock'):
        save()
        for row in original['benchmarks']:
            name = row['name']
            if Path(name).name != name or name in ('', '.', '..'):
                raise ValueError('unsafe workload name')
            source = Path(row['source'])
            if digest(source) != row['source_sha256']:
                raise ValueError(f'source changed: {name}')
            command = row['compile']['lccc']['command'].copy()
            if command.count('-o') != 1 or str(source) not in command:
                raise ValueError(f'unrecognized original command: {name}')
            binary = args.out / name / 'benchmark.bin'
            binary.parent.mkdir(exist_ok=True)
            binary.unlink(missing_ok=True)
            command[0] = str(args.compiler.resolve())
            command[command.index('-o') + 1] = str(binary.resolve())
            run = process.run(command, 180)
            error = run['failure'] or (f"exit {run['returncode']}" if run['returncode'] else None)
            if not error and not binary.is_file():
                error = 'missing executable'
            actual = None if error else digest(binary)
            check = dict(name=name, command=command, original_sha256=hashes[name]['lccc'],
                         current_sha256=actual, error=error,
                         stderr=run['stderr'].decode('utf-8', errors='backslashreplace'))
            result['checks'].append(check)
            if error:
                result['failures'].append(name)
            elif actual != hashes[name]['lccc']:
                result['changed'].append(name)
            save()
        result['complete'] = True
        save()
    print(json.dumps({k: result[k] for k in ['complete', 'changed', 'failures']}))
    return int(bool(result['failures']))


if __name__ == '__main__':
    raise SystemExit(main())
