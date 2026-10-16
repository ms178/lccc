#!/usr/bin/env python3
"""Bounded codegen survey preserving benchmark aliases and per-case flags.

A benchmark id is not necessarily a source filename. In particular, scaled
find-bit and match-rich LZ4 require different defines from their source's default
workload. Reuse the benchmark registry and existing Compiler Explorer driver;
never present repeated default-source compilations as coverage of these variants.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tests/benchmark'))
import run_benchmarks as bench
sys.path.insert(0, str(ROOT))
from tools.corpus import publication


def select_cases(only):
    names = only.split(',')
    registry = {case.name: case for case in bench.BENCHMARKS}
    if len(set(names)) != len(names) or not names or any(n not in registry for n in names):
        raise ValueError('empty, duplicate or unknown benchmark selection')
    return [registry[name] for name in names]


def case_contract(case, common_flags):
    source = bench.source_for(case)
    if source is None:
        raise ValueError('missing source for ' + case.name)
    return dict(name=case.name, source=str(source),
                source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),
                extra_flags=list(case.extra_flags),
                flags=shlex.join([*shlex.split(common_flags), *case.extra_flags]))


def validate_result(data, source, keys, expected_flags):
    if not isinstance(data, list) or len(data) != 1:
        raise ValueError('expected exactly one source result')
    row = data[0]
    if not isinstance(row, dict):
        raise ValueError('malformed source result')
    records = row.get('records', [])
    if not isinstance(records, list) or any(not isinstance(r, dict) for r in records):
        raise ValueError('malformed compiler records')
    if (row.get('source') != source or row.get('errors') != [] or
            len(records) != len(keys) or {r.get('key') for r in records} != set(keys)):
        raise ValueError('incomplete or mismatched compiler arms')
    expected = shlex.split(expected_flags)
    if any(shlex.split(r.get('flags', ''))[:len(expected)] != expected for r in records):
        raise ValueError('compiler flags do not preserve the benchmark contract')
    return row


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--only', required=True, help='explicit comma-separated benchmark ids')
    parser.add_argument('--local', required=True)
    parser.add_argument('--gcc', default='gcc', help='driver supplying LCCC builtin headers')
    parser.add_argument('--oracles', default='cg162,cclang2310,cicxlatest')
    parser.add_argument('--flags', default='-std=gnu17 -O2 -march=znver3 -fwrapv -ffp-contract=off')
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        cases = select_cases(args.only)
    except ValueError as error:
        parser.error(str(error))
    keys = ['lccc', *args.oracles.split(',')]
    if len(set(keys)) != len(keys) or any(not key for key in keys):
        parser.error('empty or duplicate oracle id')
    environment = {k: v for k, v in os.environ.items() if not k.startswith('CCC_')}
    include = subprocess.check_output([args.gcc, '-print-file-name=include'],
                                     text=True, env=environment, timeout=15).strip()
    if not Path(include).is_dir():
        parser.error('GCC builtin include directory does not exist')
    args.out.mkdir(parents=True, exist_ok=True)
    report = dict(complete=False, metric='static codegen, not execution or speed',
                  selected=[case.name for case in cases], compiler_keys=keys,
                  compiler_environment_policy='remove inherited CCC_*', cases=[], failures=[])

    def save():
        publication.atomic_bytes(args.out / 'index.json', (json.dumps(report, indent=2) + '\n').encode())

    with publication.lock(args.out / '.survey.lock'):
        save()
        for case in cases:
            entry = case_contract(case, args.flags)
            directory = (args.out / case.name).resolve()
            directory.mkdir(parents=True, exist_ok=True)
            result = directory / 'result.json'
            result.unlink(missing_ok=True)  # a failed retry must not accept old output
            command = [sys.executable, str(ROOT / 'scripts/codegen_oracle.py'), entry['source'],
                       '--local', args.local, '--oracles', args.oracles, '--flags', entry['flags'],
                       '--local-flags', entry['flags'] + ' ' + shlex.quote('-I' + include),
                       '--artifact-dir', str(directory / 'artifacts'), '--json', str(result), '-j', '2']
            entry['command'] = command
            try:
                with (directory / 'run.log').open('w') as log:
                    run = subprocess.run(command, stdout=log, stderr=log, env=environment, timeout=240)
                if run.returncode:
                    raise ValueError('oracle driver exited ' + str(run.returncode))
                entry['result'] = validate_result(json.loads(result.read_text()), entry['source'], keys, entry['flags'])
            except (OSError, ValueError, subprocess.TimeoutExpired) as error:
                entry['error'] = str(error)
                report['failures'].append(case.name)
            report['cases'].append(entry)
            save()
        report['complete'] = True
        save()
    print(f"{len(cases)} benchmark contracts; {len(report['failures'])} failures")
    return int(bool(report['failures']))


if __name__ == '__main__':
    raise SystemExit(main())
