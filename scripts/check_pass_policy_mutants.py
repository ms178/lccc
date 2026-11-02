#!/usr/bin/env python3
"""Execute the pure policy unit and kill both independent flag-setter mutants.

The integration test separately runs run_passes across all four policy pairs
and O0..O3, sequential TUs, and a worker handoff. This test does not count a
compile failure as killing a mutant: every variant must compile and then the
mutants must fail the same executable assertion that the original passes.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument('--json', type=Path, required=True)
    args = parser.parse_args()
    source = ROOT / 'src/common/pass_policy.rs'
    raw = source.read_text()
    variants = {'original': raw}
    for field in ('wrap_signed_overflow', 'fp_environment_observable'):
        needle = '            ' + field + ','
        if raw.count(needle) != 1:
            parser.error('constructor mutation site is not unique: ' + field)
        variants[field] = raw.replace(needle, '            ' + field + ': false,')
    report = {'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
              'rustc': subprocess.check_output(['rustc', '-Vv'], text=True),
              'complete': False, 'rows': []}
    args.json.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='lccc-policy-mutants-') as td:
        root = Path(td)
        for label, text in variants.items():
            src, binary = root / 'policy.rs', root / 'policy'
            src.write_text(text)
            build_cmd = ['rustc', '--edition=2024', '--test', '-C', 'opt-level=1',
                         '-A', 'unused-variables', str(src), '-o', str(binary)]
            build = subprocess.run(build_cmd, capture_output=True, text=True, timeout=120)
            row = {'variant': label, 'build_command': build_cmd,
                   'build_status': build.returncode, 'build_stderr': build.stderr}
            report['rows'].append(row)
            if build.returncode:
                break
            run = subprocess.run([str(binary), '--exact', 'tests::flags_are_independent'],
                                 capture_output=True, text=True, timeout=30)
            row.update(run_status=run.returncode, stdout=run.stdout, stderr=run.stderr,
                       pass_verdict=(run.returncode == 0 if label == 'original'
                                     else run.returncode == 101 and 'assertion' in run.stdout))
        report['complete'] = (len(report['rows']) == 3
                              and all(r.get('pass_verdict') for r in report['rows']))
    tmp = args.json.with_suffix('.tmp')
    tmp.write_text(json.dumps(report, indent=2) + '\n')
    os.replace(tmp, args.json)
    print(('PASS' if report['complete'] else 'FAIL') + ': original policy and both setter mutants')
    return int(not report['complete'])


if __name__ == '__main__':
    raise SystemExit(main())
