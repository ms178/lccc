#!/usr/bin/env python3
"""Deterministic binary32/64/128 and x87 hex-literal GCC/Clang and Memcheck oracle.

Long significands, distant sticky bits, leading zeros, exponent extremes and
normal/subnormal boundaries. Compare the emitted object data, not host float
parsing or a reference compiler's constant-folded arithmetic.
"""
import argparse
import hashlib
import json
from pathlib import Path
import random
import shutil
import subprocess
import tempfile


def source():
    rng = random.Random(8331074)
    groups = []
    for suffix, emin, emax in [('f', -149, 127), ('', -1074, 1023), ('Q', -16494, 16383), ('L', -16445, 16383)]:
        literals = ['0x0p0', '0x1p0', f'0x1p{emin}', f'0x1p{emin-1}', f'0x3p{emin-1}',
                    '0x1p99999999999999999999', '0x1p-99999999999999999999',
                    '0x100000000000000000000000000000000p-128',
                    '0x000000000000000000000000000000000000001p0']
        for fraction in [23, 52, 63, 112]:
            # Exact halfway value and a tail too remote for binary128 to retain.
            half = (1 << (fraction + 1)) + 1
            literals.extend([f'0x{half:x}p-{fraction+1}',
                             f'0x{half:x}{"0"*80}1p-{fraction+1+324}'])
        for i in range(120):
            count = rng.choice([1, 13, 14, 16, 17, 28, 29, 31, 32, 33, 64, 128])
            digits = f'{rng.getrandbits(4*count):0{count}x}'
            point = rng.randrange(count + 1)
            exponent = rng.choice([emin-1, emin, emin+1, -1, 0, 1, emax-1, emax, emax+1]) - 4*point
            literals.append(('-' if i % 3 == 0 else '') + f'0x{digits[:point]}.{digits[point:]}p{exponent}')
        groups.append([v + suffix for v in literals])
    pieces = ['#include <stdio.h>\n']
    types = [('float', 'unsigned int u'), ('double', 'unsigned long long u'),
             ('__float128', 'unsigned long long u[2]'), ('long double', 'unsigned long long u[2]')]
    for i, (values, (ty, field)) in enumerate(zip(groups, types)):
        pieces.append(f'static const union {{ {ty} f; {field}; }} a{i}[] = {{\n')
        pieces.extend('{.f=' + value + '},\n' for value in values)
        pieces.append('};\n')
    pieces.append('int main(void) {\n')
    for i, values in enumerate(groups):
        fmt = ['"%08x\\n", a0[i].u', '"%016llx\\n", a1[i].u',
               '"%016llx%016llx\\n", a2[i].u[1], a2[i].u[0]',
               '"%04x%016llx\\n", (unsigned)(a3[i].u[1]&65535), a3[i].u[0]'][i]
        pieces.append(f'for (unsigned i=0;i<{len(values)};++i) printf({fmt});\n')
    pieces.append('return 0; }\n')
    return ''.join(pieces), sum(map(len, groups))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--lccc', required=True)
    ap.add_argument('--gcc', default='gcc')
    ap.add_argument('--clang', default='clang')
    ap.add_argument('--out', type=Path)
    ap.add_argument('--abi', choices=['x64','x86'], default='x64')
    args = ap.parse_args()
    compilers = {k: shutil.which(getattr(args, k)) for k in ['lccc', 'gcc', 'clang']}
    # Name the missing tool.  The combined message this replaced ("LCCC, GCC,
    # Clang and Valgrind are all required") cost a CI job and a local
    # reconstruction: the gate could not say WHICH dependency the lane lacked,
    # so the red annotation pointed at the oracle rather than at provisioning.
    missing = [f'--{name}={getattr(args, name)}' for name, path in compilers.items() if not path]
    if not shutil.which('valgrind'):
        missing.append('valgrind (Memcheck; on PATH)')
    if missing:
        ap.error('missing required tool(s): ' + ', '.join(missing))
    text, count = source()
    report = {'complete': False, 'literal_count': count, 'abi': args.abi,
              'source_sha256': hashlib.sha256(text.encode()).hexdigest(), 'commands': []}
    with tempfile.TemporaryDirectory(prefix='hex-float-') as temporary:
        work = args.out.resolve() if args.out else Path(temporary)
        work.mkdir(parents=True, exist_ok=True)
        c = work / 'hex.c'
        c.write_text(text)
        def run(command):
            command = list(map(str, command))
            p = subprocess.run(command, capture_output=True, text=True, timeout=120)
            report['commands'].append(dict(command=command, returncode=p.returncode,
                                           stdout=p.stdout, stderr=p.stderr))
            if p.returncode:
                raise ValueError(f'failed {command}\n{p.stdout}\n{p.stderr}')
            return p.stdout
        try:
            report['compilers'] = {name: dict(path=path, sha256=hashlib.sha256(Path(path).read_bytes()).hexdigest(), version=run([path, '--version'])) for name, path in compilers.items()}
            outputs = {}
            for name, compiler in compilers.items():
                exe = work / name
                exe.unlink(missing_ok=True)
                run([compiler, '-O2', '-w', *(['-m32','-msse2'] if args.abi=='x86' else []), c, '-o', exe])
                outputs[name] = run([exe])
                if len(outputs[name].splitlines()) != count:
                    raise ValueError('incomplete output: ' + name)
            if len(set(outputs.values())) != 1:
                raise ValueError('literal data disagrees; inspect results.json')
            checked = run(['valgrind', '--error-exitcode=99', '--leak-check=full', work / 'lccc'])
            if checked != outputs['lccc']:
                raise ValueError('Memcheck output differs')
            report['stdout_sha256'] = hashlib.sha256(checked.encode()).hexdigest()
            report['complete'] = True
            print(f'PASS: {count} literal bit patterns; LCCC/GCC/Clang agree; Memcheck clean')
        finally:
            (work / 'results.json').write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()
