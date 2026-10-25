#!/usr/bin/env python3
"""SSE1-only maps: native exact bits, guard pages, aliases and fail-closed ISA."""
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
SOURCE = ROOT / 'tests/regression/sse1_f32_map_kernel.inc'
FIXTURE = ROOT / 'tests/regression/fp_map_unroll_bits.c'
FLAGS = ['-ffp-contract=off', '-fno-math-errno']
SSE1 = set('movss movaps movups movhps movlps movhlps movlhps unpcklps unpckhps '
           'shufps movmskps addss addps subss subps mulss mulps divss divps '
           'sqrtss sqrtps xorps andps andnps orps comiss ucomiss '
           'cvtsi2ss cvtsi2ssl cvtsi2ssq'.split())


def run(cmd, env):
    p = subprocess.run(cmd, env=env, capture_output=True, timeout=90)
    if p.returncode:
        raise RuntimeError(f'{cmd}: exit {p.returncode}\n{p.stderr.decode(errors="replace")}')
    return p.stdout


def legal(asm):
    for line in asm.splitlines():
        code = line.split('#')[0].strip()
        if not code or code.startswith('.') or code.endswith(':'):
            continue
        if re.search(r'%(?:xmm|ymm|zmm)\d+', code) and code.split()[0] not in SSE1:
            raise RuntimeError('not SSE1: ' + code)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--lccc', default=os.environ.get('CCC', str(ROOT / 'target/fastbuild/lccc')))
    p.add_argument('--gcc', default=os.environ.get('GCC', shutil.which('gcc-16') or 'gcc'))
    p.add_argument('--json', type=Path)
    args = p.parse_args()
    env = dict(os.environ)
    env.pop('CCC_NO_MAP_VEC', None)
    report = {'complete': False, 'source_sha256': hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
              'arms': [], 'negative_controls': []}
    orders = [['-mno-sse2'], ['-mno-sse2', '-msse'], ['-msse', '-mno-sse2'],
              ['-march=x86-64-v3', '-mno-sse2', '-msse']]
    try:
        with tempfile.TemporaryDirectory(prefix='sse1-map-') as td:
            td = Path(td)
            text, count = re.subn(r'real checked_fp_map\([^{}]*\) \{.*?^\}',
                                  'real checked_fp_map(real *, const real *, const real *, const real *, int, real);',
                                  FIXTURE.read_text(), count=1, flags=re.M | re.S)
            if count != 1:
                raise RuntimeError('guard harness kernel boundary changed')
            harness = td / 'harness.c'
            harness.write_text(text)
            main_obj = td / 'main.o'
            run([args.gcc, '-O2', *FLAGS, '-c', str(harness), '-o', str(main_obj)], env)
            for op in range(6):
                reference = td / 'reference.o'
                run([args.gcc, '-O2', *FLAGS, '-mno-sse2', '-msse', '-DMAP_OP=' + str(op),
                     '-x', 'c', '-c', str(SOURCE), '-o', str(reference)], env)
                binary = td / 'reference'
                run([args.gcc, str(main_obj), str(reference), '-o', str(binary), '-lm'], env)
                expected = run([str(binary)], env)
                modes = [(f'order-{i}', '-O2', flags, {}) for i, flags in enumerate(orders)]
                modes += [('scalar', '-O2', orders[0], {'CCC_NO_MAP_VEC': '1'}),
                          ('no-ra', '-O2', orders[0], {'CCC_NO_REGALLOC': '1'})]
                if op == 0:
                    modes += [(opt, opt, orders[0], {}) for opt in ['-O0', '-O1', '-O3', '-Os']]
                for name, opt, flags, extra in modes:
                    print(f'op{op}/{name}: compiling and checking', flush=True)
                    arm_env = dict(env, CCC_VALIDATE_SSA='1', **extra)
                    obj, asm = td / 'kernel.o', td / 'kernel.s'
                    cmd = [args.lccc, opt, *FLAGS, *flags, '-DMAP_OP=' + str(op), '-x', 'c']
                    run([*cmd, '-c', str(SOURCE), '-o', str(obj)], arm_env)
                    run([*cmd, '-S', str(SOURCE), '-o', str(asm)], arm_env)
                    code = asm.read_text()
                    legal(code)
                    match = re.search(r'(?ms)^checked_fp_map:\n(.*?)^\.size checked_fp_map,', code)
                    control = re.search(r'(?ms)^sse1_integer_control:\n(.*?)^\.size sse1_integer_control,', code)
                    if not match or not control or '%xmm' in control[1]:
                        raise RuntimeError('typed SSE1 admission leaked into integer code')
                    packed = bool(re.search(r'^\s+(?:add|sub|mul|div|sqrt)ps\s', match[1], re.M))
                    if opt in ['-O2', '-O3'] and name != 'scalar' and not packed:
                        raise RuntimeError(f'op{op}/{name}: missing packed F32 arithmetic')
                    if name == 'scalar' and packed:
                        raise RuntimeError('map ablation did not restore scalar code')
                    run([args.gcc, str(main_obj), str(obj), '-o', str(binary), '-lm'], env)
                    actual = run([str(binary)], arm_env)
                    if actual != expected:
                        raise RuntimeError(f'op{op}/{name}: bits/aliases differ from GCC')
                    report['arms'].append({'op': op, 'name': name, 'command': cmd, 'env': extra,
                                           'cases': len(actual.splitlines()), 'packed': packed,
                                           'sha256': hashlib.sha256(actual).hexdigest(),
                                           'assembly_sha256': hashlib.sha256(code.encode()).hexdigest()})
                print(f'op{op}: PASS {len(modes)} native/ISA arms', flush=True)
            for source, flags in [('double f(double a,double b){return a+b;}', ['-mno-sse2', '-msse']),
                                  ('float f(float a,float b){return a+b;}', ['-mno-sse'])]:
                bad = td / 'unsupported.c'
                bad.write_text(source)
                proc = subprocess.run([args.lccc, '-O2', *flags, '-S', str(bad), '-o', str(asm)],
                                      env=env, capture_output=True, timeout=90)
                if proc.returncode == 0 or b'requires SSE' not in proc.stderr:
                    raise RuntimeError('denied FP feature escaped diagnostic')
                report['negative_controls'].append({'flags': flags, 'rejected': True})
            # The legality oracle must reject SSE2 moves even on an SSE2 host.
            for instruction in ['movdqu (%rdi), %xmm0', 'pxor %xmm0, %xmm0', 'addsd %xmm1,%xmm0']:
                try:
                    legal(instruction)
                except RuntimeError:
                    continue
                raise RuntimeError('ISA mutation survived: ' + instruction)
            report['complete'] = True
    finally:
        if args.json:
            args.json.parent.mkdir(parents=True, exist_ok=True)
            args.json.write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()
