#!/usr/bin/env python3
"""Fail-closed wrapping-domain differential + Memcheck gate (24 cases).

Pass exact oracle executables explicitly; identities and commands are recorded.
Needs native and 32-bit libc/debug info. A missing Valgrind loader redirect is
an infrastructure failure, never a compiler pass or a silently skipped case.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', required=True)
    parser.add_argument('--gcc', required=True)
    parser.add_argument('--clang', required=True)
    parser.add_argument('--work-dir', type=Path, required=True)
    parser.add_argument('--json', type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    args.work_dir.mkdir(parents=True, exist_ok=True)
    args.json.parent.mkdir(parents=True, exist_ok=True)
    compilers = dict(lccc=args.lccc, gcc=args.gcc, clang=args.clang)
    report = dict(complete=False, compilers={}, cases=[])

    def save():
        temp = args.json.with_suffix('.tmp')
        temp.write_text(json.dumps(report, indent=2) + '\n')
        os.replace(temp, args.json)

    try:
        for name, cc in compilers.items():
            report['compilers'][name] = subprocess.check_output([cc, '--version'], text=True)
            for abi, flags in [('x64', []), ('x86', ['-m32', '-msse2'])]:
                binary = (args.work_dir / (name + '-' + abi)).resolve()
                cmd = [cc, '-O2', '-fwrapv', *flags, str(root / 'tests/regression/wrapv_domains.c'), '-o', str(binary)]
                build = subprocess.run(cmd, capture_output=True, text=True, timeout=60)
                if build.returncode:
                    raise RuntimeError(f'build failure: {cmd}\n{build.stderr}')
                for case, expected in enumerate(['28', '-1073665362', '1', '10']):
                    log = args.work_dir / f'{name}-{abi}-{case}.log'
                    run = subprocess.run(['valgrind', '--error-exitcode=97', '--leak-check=full', '--log-file=' + str(log), str(binary), str(case)], capture_output=True, text=True, timeout=60)
                    row = dict(compiler=name, abi=abi, case=case, build=cmd, status=run.returncode,
                               stdout=run.stdout.strip(), stderr=run.stderr, memcheck=log.read_text())
                    row['pass'] = run.returncode == 0 and row['stdout'] == expected
                    report['cases'].append(row)
                    save()
                    if not row['pass']:
                        raise RuntimeError(f'FAIL: {name}/{abi}/{case}; see {args.json}')
        report['complete'] = True
        print(f"PASS: {len(report['cases'])} exact-oracle/ABI/domain Memcheck cases")
    finally:
        save()


if __name__ == '__main__':
    main()
