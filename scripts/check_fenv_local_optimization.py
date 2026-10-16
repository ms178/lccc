#!/usr/bin/env python3
"""Native FP-effect/rounding tests plus paired Callgrind integer-work measurement.

Only the exact live-addition probe is used for cost comparisons. It does not
prove nontrivial rounding behavior; the separate effect executable does that.
GCC's pragma support differs, so GCC is not the reference for dead FP effects.
"""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def command(args):
    argv = [str(a) for a in args]
    try:
        p = subprocess.run(argv, capture_output=True, text=True, timeout=120)
        return {'command': argv, 'status': p.returncode,
                'stdout': p.stdout, 'stderr': p.stderr}
    except (OSError, subprocess.TimeoutExpired) as error:
        return {'command': argv, 'status': 'execution-error', 'stdout': '', 'stderr': str(error)}


def main():
    ap = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    ap.add_argument('--compiler', type=Path, required=True)
    ap.add_argument('--reference', type=Path)
    ap.add_argument('--baseline', type=Path, help='record failures without accepting them as current results')
    ap.add_argument('--output', type=Path, required=True)
    ap.add_argument('--json', type=Path, required=True)
    ap.add_argument('--native-only', action='store_true', help='correctness gate only; no performance claim')
    a = ap.parse_args()
    a.output.mkdir(parents=True, exist_ok=True)
    report = {'complete': False, 'effects': [], 'work': [], 'sources': {}, 'compilers': {},
              'native_only': a.native_only}
    a.json.parent.mkdir(parents=True, exist_ok=True)
    def save():
        temporary = a.json.with_suffix('.tmp')
        temporary.write_text(json.dumps(report, indent=2) + '\n')
        temporary.replace(a.json)
    save()
    effects = ROOT / 'tests/optimizer/fenv_dead_effects.c'
    work = ROOT / 'tests/optimizer/fenv_policy_work.c'
    for path in (effects, work):
        report['sources'][str(path.relative_to(ROOT))] = sha(path)
    ok = True
    for label, compiler in [('baseline', a.baseline), ('reference', a.reference), ('candidate', a.compiler)]:
        if compiler is None:
            continue
        compiler = compiler.resolve()
        report['compilers'][label] = {'path': str(compiler), 'sha256': sha(compiler)}
        for level in range(4):
            binary = a.output / f'{label}-effects-O{level}'
            binary.unlink(missing_ok=True)
            build = command([compiler, f'-O{level}', '-frounding-math', '-ffp-contract=off', effects, '-lm', '-o', binary])
            row = {'compiler': label, 'level': level, 'build': build}
            if build['status'] == 0:
                row['run'] = command([binary])
            row['pass'] = (row.get('run', {}).get('status') == 0 and
                           row['run']['stdout'] == 'fenv-dead-effects and four rounding modes: PASS\n')
            if label != 'baseline':
                ok &= row['pass']
            report['effects'].append(row)
            save()
        if label == 'reference' or a.native_only:
            continue
        for observe in (0, 1):
            binary = a.output / f'{label}-work-{observe}'
            profile = a.output / f'{label}-work-{observe}.callgrind'
            binary.unlink(missing_ok=True)
            profile.unlink(missing_ok=True)
            build = command([compiler, '-O2', '-fwrapv', '-ffp-contract=off', f'-DOBSERVE_FENV={observe}', work, '-lm', '-o', binary])
            row = {'compiler': label, 'observe_fenv': observe, 'build': build}
            if build['status'] == 0:
                row['native'] = command([binary])
                row['callgrind'] = command(['valgrind', '--tool=callgrind', '--error-exitcode=99', f'--callgrind-out-file={profile}', binary])
                if profile.exists():
                    totals = [int(line.split()[1]) for line in profile.read_text().splitlines() if line.startswith('summary:')]
                    if len(totals) == 1:
                        row['Ir'] = totals[0]
                        row['profile_sha256'] = sha(profile)
            expected = '62500.0\n' * 4
            row['pass'] = ('Ir' in row and all(row.get(k, {}).get('status') == 0
                          and row[k]['stdout'] == expected for k in ('native', 'callgrind')))
            ok &= row['pass']
            report['work'].append(row)
            save()
    report['complete'] = bool(ok)
    save()
    print(('PASS' if ok else 'FAIL') + ': native FP effects and rounding' +
          ('' if a.native_only else ', paired Callgrind'))
    for row in report['effects']:
        print(row['compiler'], 'O'+str(row['level']), 'PASS' if row['pass'] else 'FAIL', row.get('run', {}).get('status'))
    for row in report['work']:
        print(row['compiler'], 'FENV='+str(row['observe_fenv']), 'Ir='+str(row.get('Ir')))
    return int(not ok)


if __name__ == '__main__':
    raise SystemExit(main())
