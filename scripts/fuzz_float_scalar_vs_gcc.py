#!/usr/bin/env python3
"""Scalar float differential fuzz: lccc vs GCC 16.2 on x86-64.

Each program keeps 10-18 float values live across a loop of non-inlined
float calls, float<->u32 bit casts (memcpy), compares feeding phis, and
guarded float->int conversions, so values are spilled, reloaded, carried
around back edges, homed in GPRs and coalesced into FP copy webs. Output is
an integer hash of the bit patterns: any divergence is a wrong-code bug.

Modes:
  sse1     x86-64 ``-mno-sse2``: every scalar float route must stay SSE1
           (bridges instead of MOVD/MOVQ/MOVSD); lccc at -O0/-O2/-O2
           -fomit-frame-pointer/-O3 vs ``gcc-16 -O2 -mno-sse2``.
  default  baseline ISA; lccc at -O1/-O2/-O3/-O2 -mavx2 vs ``gcc-16 -O2``.

Every program is screened with UBSan including float-cast-overflow before
it counts (an out-of-range float->int conversion is UB, and a divergence
there is not a compiler bug). Bugs this found: the SSE1 scalar routes of
#980 that still emitted SSE2 moves, and the FP copy-web leader that took a
caller-saved XMM across a call (seeds 197 and 203 in default mode).

Usage:
    fuzz_float_scalar_vs_gcc.py [--mode sse1|default] [--seed S] [--cases N]
                                [--lccc PATH] [--gcc PATH] [--keep DIR]
Exit status is 0 iff every case agrees and none is refused.
"""

import argparse
import os
import random
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MODES = {
    'sse1': (['-O2', '-mno-sse2'],
             [['-O0'], ['-O2'], ['-O2', '-fomit-frame-pointer'], ['-O3']], ['-mno-sse2']),
    'default': (['-O2'], [['-O1'], ['-O2'], ['-O3'], ['-O2', '-mavx2']], []),
}


def gen(seed):
    r = random.Random(seed)
    nv = r.randint(10, 18)
    L = []
    L.append('#include <stdio.h>\n#include <string.h>\n')
    L.append('static unsigned bits(float f){unsigned u;memcpy(&u,&f,4);return u;}\n')
    L.append('static float fromb(unsigned u){float f;memcpy(&f,&u,4);return f;}\n')
    L.append('__attribute__((noinline)) float h1(float a, int k, float b){return k&1 ? a*b+0.5f : a-b;}\n')
    L.append('__attribute__((noinline)) float h2(float a, float b, float c, float d){return (a+b)*(c-d);}\n')
    L.append('__attribute__((noinline)) unsigned leaf(float a, float b, unsigned m){\n')
    L.append('  float t=a*b; unsigned u=bits(t)^m; float s=fromb(u&0x3f7fffffu); return bits(s+a) + (a<b) + (t>=s)*2u;}\n')
    L.append('__attribute__((noinline)) unsigned f(float *in, unsigned *mi, int n){\n')
    for i in range(nv):
        L.append(f'  float v{i} = in[{i % 8}] * {r.uniform(-3, 3):.3f}f + {r.uniform(-2, 2):.3f}f;\n')
    L.append('  unsigned acc = 0;\n  for (int it = 0; it < n; it++) {\n')
    for _ in range(r.randint(12, 24)):
        a, b, c = (r.randrange(nv) for _ in range(3))
        k = r.randrange(8)
        if k == 0: L.append(f'    v{a} = v{b} * v{c} + 0.25f;\n')
        elif k == 1: L.append(f'    v{a} = h1(v{b}, it, v{c});\n')
        elif k == 2: L.append(f'    v{a} = h2(v{b}, v{c}, v{a}, (float)it);\n')
        elif k == 3: L.append(f'    acc += bits(v{a}) >> {r.randrange(1, 9)};\n')
        elif k == 4: L.append(f'    v{a} = fromb((bits(v{b}) & 0x807fffffu) | 0x3f000000u);\n')
        elif k == 5: L.append(f'    acc += leaf(v{b}, v{c}, mi[it & 3]);\n')
        elif k == 6: L.append(f'    if (v{a} < v{b}) v{c} = v{a} - v{b}; else acc ^= bits(v{c});\n')
        else: L.append(f'    {{ float q = v{b} * 4.0f; v{a} = (q > -1.0e9f && q < 1.0e9f) ? (float)(int)q * 0.125f : q; }}\n')
    L.append('  }\n')
    for i in range(nv):
        L.append(f'  acc = acc * 31u + bits(v{i});\n')
    L.append('  return acc;\n}\n')
    L.append('int main(void){ float in[8]; unsigned mi[4]={0x1234u,0xffu,0x80000000u,7u};\n')
    L.append('  for(int i=0;i<8;i++) in[i]=(float)(i*7%5)-1.5f+i*0.125f;\n')
    L.append('  printf("%08x\\n", f(in, mi, 37)); return 0; }\n')
    return ''.join(L)


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, timeout=60, **kw)


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--mode', choices=sorted(MODES), default='sse1')
    ap.add_argument('--seed', type=int, default=1, help='first seed')
    ap.add_argument('--cases', type=int, default=120)
    ap.add_argument('--lccc', default=os.environ.get('CCC', str(ROOT / 'target/fastbuild/lccc')))
    ap.add_argument('--gcc', default=os.environ.get('GCC', shutil.which('gcc-16') or 'gcc-16'))
    ap.add_argument('--keep', type=Path, help='copy diverging programs here')
    args = ap.parse_args()
    oracle_flags, lccc_levels, extra = MODES[args.mode]
    ok = bad = refused = ub = 0
    with tempfile.TemporaryDirectory(prefix='fuzz-float-') as tmp:
        tmp = Path(tmp)
        src, exe = tmp / 'case.c', tmp / 'case'
        for seed in range(args.seed, args.seed + args.cases):
            src.write_text(gen(seed))
            screen = run([args.gcc, '-O0', '-w', '-fsanitize=undefined,float-cast-overflow',
                          '-fno-sanitize-recover=all', str(src), '-o', str(exe)])
            if screen.returncode or run([str(exe)]).returncode:
                ub += 1
                continue
            if run([args.gcc, *oracle_flags, '-w', str(src), '-o', str(exe)]).returncode:
                raise SystemExit(f'oracle failed to build seed {seed}')
            want = run([str(exe)]).stdout
            for level in lccc_levels:
                env = dict(os.environ, CCC_VERIFY_IR='abort')
                built = run([args.lccc, *level, *extra, '-w', str(src), '-o', str(exe)], env=env)
                if built.returncode:
                    refused += 1
                    print(f'REFUSED seed {seed} {" ".join(level)}: {built.stderr.strip()[:200]}')
                    continue
                got = run([str(exe)]).stdout
                if got == want:
                    ok += 1
                    continue
                bad += 1
                print(f'MISMATCH seed {seed} {" ".join(level)}: {got.strip()} vs {want.strip()}')
                if args.keep:
                    args.keep.mkdir(parents=True, exist_ok=True)
                    shutil.copy(src, args.keep / f'{args.mode}-{seed}.c')
    print(f'{args.mode}: ok={ok} mismatch={bad} refused={refused} ub_skipped={ub}')
    return 0 if bad == 0 and refused == 0 else 1


if __name__ == '__main__':
    sys.exit(main())
