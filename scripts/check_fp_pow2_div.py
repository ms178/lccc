#!/usr/bin/env python3
"""Strict power-of-two division: non-vacuous codegen and native MXCSR oracle.

Use a common GCC -O0 driver containing volatile reference divisions. Check
4928 result/exception pairs, four rounding modes and all FTZ/DAZ combinations.
Valgrind is not a floating-exception oracle; run this check natively.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import godbolt

ROOT = Path(__file__).resolve().parents[1]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--lccc', required=True)
    ap.add_argument('--gcc', default='gcc')
    ap.add_argument('--out', type=Path)
    args = ap.parse_args()
    cc = shutil.which(args.lccc)
    gcc = shutil.which(args.gcc)
    if not cc or not gcc:
        ap.error('both compiler executables are required')
    source = ROOT / 'tests/fixtures/fp_pow2/kernels.c'
    driver = ROOT / 'tests/fixtures/fp_pow2/driver.c'
    report = {'complete': False, 'commands': [], 'checks': {}, 'sources': {
        str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in [source, driver]}}
    with tempfile.TemporaryDirectory(prefix='fp-pow2-') as temporary:
        work = args.out.resolve() if args.out else Path(temporary)
        work.mkdir(parents=True, exist_ok=True)
        def run(command):
            command = [str(x) for x in command]
            p = subprocess.run(command, capture_output=True, text=True, timeout=120)
            report['commands'].append(dict(command=command, returncode=p.returncode,
                                           stdout=p.stdout, stderr=p.stderr))
            if p.returncode:
                raise ValueError(f'command failed: {command}\n{p.stdout}\n{p.stderr}')
            return p.stdout
        try:
            report['compilers'] = {name: dict(path=path, sha256=hashlib.sha256(Path(path).read_bytes()).hexdigest(), version=run([path, '--version'])) for name, path in {'lccc': cc, 'gcc': gcc}.items()}
            for name in ['kernel.s', 'kernel.o', 'driver.o', 'probe']:
                (work / name).unlink(missing_ok=True)
            flags = ['-O2', '-march=x86-64-v3', '-ffp-contract=off']
            run([cc, *flags, '-S', source, '-o', work / 'kernel.s'])
            lines = (work / 'kernel.s').read_text().splitlines()
            names = [n for n in re.findall(r'D(?:32|64)\((\w+),', source.read_text())
                     if n.startswith(('div', 'reject'))]
            if len(names) != 22:
                raise ValueError('unexpected witness inventory')
            for name in names:
                body = godbolt._function_body(lines, name)
                if body is None:
                    raise ValueError('missing function: ' + name)
                text = '\n'.join(body)
                div = len(re.findall(r'\bv?divs[sd]\b', text))
                mul = len(re.findall(r'\bv?muls[sd]\b', text))
                add = len(re.findall(r'\bv?adds[sd]\b', text))
                # The existing exact x*2 -> x+x lowering is also valid; only
                # the positive-half divisors can take that route here.
                wanted = {(1, 0, 0)} if name.startswith('reject') else {(0, 1, 0)}
                if name.endswith('_half'):
                    wanted.add((0, 0, 1))
                report['checks'][name] = dict(div=div, mul=mul, add=add, expected=sorted(wanted))
                if (div, mul, add) not in wanted:
                    raise ValueError(f'non-vacuity/safety shape failure: {name}: {(div,mul,add)} not in {wanted}')
            run([cc, '-c', work / 'kernel.s', '-o', work / 'kernel.o'])
            run([gcc, '-O0', '-frounding-math', '-fsignaling-nans', '-c', driver, '-o', work / 'driver.o'])
            run([gcc, work / 'kernel.o', work / 'driver.o', '-lm', '-o', work / 'probe'])
            output = run([work / 'probe'])
            if output != 'POW2_PASS 4928\n':
                raise ValueError('unexpected oracle result: ' + output)
            report['complete'] = True
            print('PASS: 22 codegen contracts; 4928 native bit/exception comparisons')
        finally:
            (work / 'results.json').write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()
