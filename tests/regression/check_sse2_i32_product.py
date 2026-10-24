#!/usr/bin/env python3
"""ISA legality + guard-page/overflow oracle for packed I32 low products.

Native equality on a modern CPU cannot detect an SSE4.1 leak into SSE2 code.
Check instruction legality independently, and require actual packed products
so that silently disabling vectorization cannot make this gate pass.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / 'tests/regression/sse2_i32_product.c'
COMMON = ['-O2', '-fwrapv', '-ffp-contract=off']
# Deliberately fail closed on new SIMD instructions in these small kernels.
# Scalar/SIMD register transfers are included; cvtsi2sdl is AT&T's I32 form.
SSE2_KERNEL_OPS = {
    'movd', 'movq', 'movdqa', 'movdqu', 'movapd', 'movupd', 'movaps', 'movups',
    'movsd', 'movss', 'movlpd', 'movhpd', 'paddd', 'paddq', 'pxor', 'por',
    'pand', 'pandn', 'pmuludq', 'pshufd', 'psrldq', 'pslldq', 'psrlq',
    'pslld', 'psrld', 'psrad', 'punpckldq', 'punpckhdq', 'punpcklqdq',
    'punpckhqdq', 'addsd', 'subsd', 'mulsd', 'divsd', 'cvtsi2sdl', 'cvtsi2sdq',
}



def run(command, env):
    p = subprocess.run(command, env=env, capture_output=True, timeout=90)
    if p.returncode:
        raise RuntimeError(f'{command}: exit {p.returncode}\n{p.stderr.decode(errors="replace")}')
    return p.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default=os.environ.get('CCC', str(ROOT / 'target/fastbuild/lccc')))
    parser.add_argument('--gcc', default=os.environ.get('GCC', shutil.which('gcc-16') or 'gcc'))
    parser.add_argument('--clang', default=os.environ.get('CLANG', shutil.which('clang-23')))
    parser.add_argument('--json', type=Path)
    args = parser.parse_args()
    env = dict(os.environ)
    report = {'complete': False, 'source_sha256': hashlib.sha256(SOURCE.read_bytes()).hexdigest(), 'arms': [], 'mutations': []}
    expected = None
    modes = [('gcc', args.gcc, ['-march=x86-64'], {}, 'oracle')]
    if args.clang:
        modes.append(('clang', args.clang, ['-march=x86-64'], {}, 'oracle'))
    profiles = [
        ('default', [], 'sse2'),
        ('sse2', ['-march=x86-64'], 'sse2'),
        ('sse3', ['-march=x86-64', '-msse3'], 'sse2'),
        ('ssse3', ['-march=x86-64', '-mssse3'], 'sse2'),
        ('sse41', ['-march=x86-64', '-msse4.1'], 'sse41'),
        ('sse42', ['-march=x86-64-v2'], 'sse41'),
        ('avx', ['-march=x86-64-v2', '-mavx'], 'sse41'),
        ('avx2', ['-march=x86-64-v3'], 'avx2'),
        ('sse41-denied', ['-march=x86-64-v2', '-mno-sse4.1'], 'sse2'),
    ]
    for name, flags, isa in profiles:
        modes.append((name, args.lccc, flags, {}, isa))
    for name, extra in [
        ('no-xmm', {'CCC_NO_XMM_REGALLOC': '1'}),
        ('no-vecreg', {'CCC_NO_VECREG': '1'}),
        ('no-homes', {'CCC_NO_XMM_REGALLOC': '1', 'CCC_NO_VECREG': '1'}),
        ('no-promote', {'CCC_NO_VEX_PROMOTE': '1'}),
    ]:
        modes.append((name, args.lccc, ['-march=x86-64'], extra, 'sse2'))
    try:
        with tempfile.TemporaryDirectory(prefix='sse2-product-') as td:
            for name, compiler, flags, extra, isa in modes:
                arm_env = dict(env, **extra)
                if isa != 'oracle':
                    arm_env['CCC_VALIDATE_SSA'] = '1'
                binary = Path(td) / name
                command = [compiler, *COMMON, *flags, str(SOURCE), '-o', str(binary)]
                run(command, arm_env)
                output = run([str(binary)], arm_env)
                if expected is None:
                    expected = output
                if output != expected:
                    raise RuntimeError(f'{name}: stdout differs from GCC (both exited zero)')
                row = {'name': name, 'command': command, 'env': extra,
                       'version': run([compiler, '--version'], arm_env).decode(),
                       'cases': len(output.splitlines()), 'sha256': hashlib.sha256(output).hexdigest()}
                if isa != 'oracle':
                    assembly = binary.with_suffix('.s')
                    run([compiler, *COMMON, *flags, '-S', str(SOURCE), '-o', str(assembly)], arm_env)
                    text = assembly.read_text()
                    bodies = []
                    for function in ['product_dot', 'product_dual', 'product_seeded', 'product_map', 'product_lanes']:
                        match = re.search(rf'(?ms)^{function}:\n(.*?)^\.size {function},', text)
                        if not match:
                            raise RuntimeError(f'{name}: missing {function}')
                        body = match.group(1)
                        ops = re.findall(r'^\s+([a-z][a-z0-9]*)\s', body, re.M)
                        products = sum(op in ('pmuludq', 'pmulld', 'vpmuludq', 'vpmulld') for op in ops)
                        if not products:
                            raise RuntimeError(f'{name}/{function}: packed multiply disappeared')
                        if isa == 'sse2':
                            simd_ops = {re.match(r'\s*([a-z0-9]+)', line)[1]
                                        for line in body.splitlines()
                                        if '%xmm' in line and re.match(r'\s*[a-z][a-z0-9]*\s', line)}
                            forbidden = simd_ops - SSE2_KERNEL_OPS
                            if forbidden or any(op.startswith('v') for op in ops):
                                raise RuntimeError(f'{name}/{function}: non-SSE2 instructions: {forbidden or ops}')
                            if products < 2 or 'punpckldq' not in ops:
                                raise RuntimeError(f'{name}/{function}: missing odd/even product reconstruction')
                        elif 'pmulld' not in ops and 'vpmulld' not in ops:
                            raise RuntimeError(f'{name}/{function}: SSE4.1/AVX2 direct multiply lost')
                        bodies.append(body)
                    row['assembly'] = '\n'.join(bodies)
                    if name == 'sse2':
                        # A dot product alone cannot observe a permutation of
                        # products. The lane-map hashes must also kill the
                        # swapped-even/odd mutant, not just wrong arithmetic.
                        for label, pattern, replacement in [
                            ('wrong-odd', r'pshufd \$0xf5,', 'pshufd $0xf0,'),
                            ('swapped-lanes', r'punpckldq %xmm2, %(xmm\d+)',
                             r'punpckldq %\1, %xmm2\n    movdqa %xmm2, %\1'),
                        ]:
                            mutated, sites = re.subn(pattern, replacement, text)
                            if not sites:
                                raise RuntimeError(f'{label}: no mutation sites')
                            mutant = Path(td) / label
                            source = mutant.with_suffix('.s')
                            source.write_text(mutated)
                            run([args.lccc, str(source), '-o', str(mutant)], arm_env)
                            wrong = run([str(mutant)], arm_env)
                            if wrong == expected:
                                raise RuntimeError(f'{label}: mutant survived')
                            report['mutations'].append({'name': label, 'sites': sites,
                                                        'exit': 0, 'output_mismatch': True})
                            print(f'{label}: KILLED ({sites} sites, native exit zero)')
                report['arms'].append(row)
                print(f'{name}: PASS {row["cases"]} exact-output cases + {isa} legality')
            report['complete'] = True
    finally:
        if args.json:
            args.json.parent.mkdir(parents=True, exist_ok=True)
            args.json.write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()
