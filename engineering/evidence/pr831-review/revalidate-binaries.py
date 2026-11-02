#!/usr/bin/env python3
"""Recompile recorded benchmark command pairs and compare complete ELF bytes.

This is evidence continuity, not a new timing measurement. Both tool binaries
must exist. The original manifest records the exact flags; source/output paths
are normalized to the current checkout and a private temporary directory.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from tests.benchmark import run_benchmarks as registry


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--before', type=Path, required=True)
    p.add_argument('--after', type=Path, required=True)
    p.add_argument('--manifest', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    original = json.loads(a.manifest.read_text())
    cases = {case.name: case for case in registry.BENCHMARKS}
    report = {'old_compiler': digest(a.before), 'new_compiler': digest(a.after),
              'complete': False, 'comparisons': []}
    with tempfile.TemporaryDirectory(prefix='lccc-identity-') as td:
        for row in original['comparisons']:
            case = cases[row['case']]
            hashes, commands = [], []
            for tag, binary in [('before', a.before), ('after', a.after)]:
                out = Path(td) / tag
                old = row['commands'][0]
                if isinstance(old, dict):
                    import shlex
                    command = [str(binary.resolve()), *shlex.split(old['flags']),
                               '-I/usr/lib/gcc/x86_64-linux-gnu/16/include',
                               str(registry.source_for(case)), '-o', str(out)]
                else:
                    command = list(old)
                    command[0] = str(binary.resolve())
                    command[command.index('-o') + 1] = str(out)
                    for i, value in enumerate(command):
                        if value.endswith('.c'):
                            command[i] = str(registry.source_for(case))
                result = subprocess.run(command, capture_output=True, text=True, timeout=60)
                if result.returncode:
                    raise RuntimeError(f'{command}: {result.stderr}')
                hashes.append(digest(out)); commands.append(command)
            report['comparisons'].append({'case': row['case'], 'profile': row['profile'],
                                         'before_sha256': hashes[0], 'after_sha256': hashes[1],
                                         'identical': hashes[0] == hashes[1], 'commands': commands})
    report['complete'] = True
    a.output.write_text(json.dumps(report, indent=2) + '\n')
    changed = [(r['case'], r['profile']) for r in report['comparisons'] if not r['identical']]
    print(f'{len(report["comparisons"])} comparisons; changed={changed}')
    return bool(changed)


if __name__ == '__main__':
    sys.exit(main())
