#!/usr/bin/env python3
"""Boundary differential fuzzer for counted loops, byte counts, strict
reciprocals and signed-I32-at-pointer-width codegen.

Every emitted translation unit is free of undefined behaviour by
construction: each loop trajectory is simulated here in exact integer
arithmetic before it is emitted (no signed overflow unless the TU is a
-fwrapv TU, no out-of-bounds index, guaranteed termination), and every
expression family is range-bounded. Two reference builds (GCC -O0 and
GCC -O2, same -f flags) must agree before LCCC is judged at all: a reference
disagreement is reported as a GENERATOR defect, never as an LCCC verdict.

Families (all deliberately aimed at boundary values):
  bound    unsigned u32/u64 loops `(T)(i + k) CMP limit` with constant
           init/step/offset/limit near 0 and the type maximum, every
           comparison spelling (both operand orders, inclusive/exclusive,
           continue-on-true and exit-on-true), optional early break.
  runtime  the same trajectories with init/limit loaded through volatile
           globals (late affine fold, iv_widen, unroller remainders).
  wrapv    signed int/long loops; in -fwrapv TUs trajectories may wrap.
  bytecnt  byte predicate counts over [start, end) with nonzero start,
           every counter width (including modular u8/u16 counters),
           signed/unsigned char predicates, lengths around 16/32/256.
  recip    strict FP reciprocal-coefficient DAGs (spectral-norm shape)
           over special values (NaN, inf, -0.0, subnormals), printed as
           exact bit patterns.
  sext     signed I32 values (negative included) feeding pointer-width
           indexing, long conversions, memory RMW and phi joins.

Usage:
  boundary_loop_fuzz.py --lccc PATH [--gcc gcc-16] [--seeds 0:40]
      [--levels O1,O2,O3] [--march x86-64,x86-64-v3] [--out DIR] [--keep]
Exit status: 0 = no LCCC mismatch, 1 = LCCC mismatch, 2 = harness/generator
defect (reference disagreement, build failure of the reference).
"""
from __future__ import annotations

import argparse
import json
import random
import subprocess
import sys
from pathlib import Path

CAP = 4000  # maximum simulated trip count per emitted loop

U32 = (32, False)
U64 = (64, False)
I32 = (32, True)
I64 = (64, True)


def ctype(kind):
    bits, signed = kind
    return {(32, False): 'unsigned', (64, False): 'unsigned long',
            (32, True): 'int', (64, True): 'long'}[kind]


def wrap(v, kind):
    bits, signed = kind
    v &= (1 << bits) - 1
    if signed and v >> (bits - 1):
        v -= 1 << bits
    return v


def in_range(v, kind):
    bits, signed = kind
    lo, hi = (-(1 << (bits - 1)), (1 << (bits - 1)) - 1) if signed else (0, (1 << bits) - 1)
    return lo <= v <= hi


def lit(v, kind):
    bits, signed = kind
    if signed:
        if v == -(1 << (bits - 1)):
            return f'({ctype(kind)})({v + 1} - 1)'
        return f'({ctype(kind)}){v}' + ('L' if bits == 64 else '')
    return f'({ctype(kind)}){v}u' + ('l' if bits == 64 else '')


CMPS = {
    '<': lambda a, b: a < b, '<=': lambda a, b: a <= b,
    '>': lambda a, b: a > b, '>=': lambda a, b: a >= b,
    '!=': lambda a, b: a != b,
}


def boundary_values(rng, kind):
    bits, signed = kind
    mx = (1 << (bits - 1)) - 1 if signed else (1 << bits) - 1
    mn = -(1 << (bits - 1)) if signed else 0
    picks = [mn, mn + 1, mn + 2, 0, 1, 2, 3, 7, 8, 15, 16, 17, 255, 256,
             mx, mx - 1, mx - 2, mx - 3, mx - 7, mx - 15, mx - 16, mx - 17,
             mx // 2, mx // 2 + 1, (mx >> 1) - 1]
    v = rng.choice(picks) if rng.random() < 0.75 else rng.randint(mn, mx)
    return max(mn, min(mx, v))


def simulate(kind, init, step, k, limit, cmp, cont_on_true, brk, wrapv):
    """Return (trips, acc, cnt) or None when not emit-safe."""
    i = init
    acc = 0
    cnt = 0
    for _ in range(CAP + 1):
        s = i + k
        if not in_range(s, kind):
            if kind[1] and not wrapv:
                return None  # signed overflow in the header: UB
            s = wrap(s, kind)
        t = CMPS[cmp](s, limit)
        if t != cont_on_true:
            return cnt, acc, i
        if brk is not None and i == brk:
            return cnt, acc, i
        acc = (acc * 31 + (i & 0xFFFFFFFF)) & 0xFFFFFFFFFFFFFFFF
        cnt += 1
        n = i + step
        if not in_range(n, kind):
            if kind[1] and not wrapv:
                return None
            n = wrap(n, kind)
        i = n
    return None  # does not terminate within CAP: not emitted


def gen_bound(rng, idx, wrapv, runtime):
    for _ in range(400):
        kind = rng.choice([U32, U64, I32, I64] if wrapv else [U32, U64, U32, U64, I32, I64])
        init = boundary_values(rng, kind)
        step = rng.choice([1, 1, 1, 2, 3, 4, 7, 8, 16, 255])
        k = rng.choice([1, 1, 2, 3, 4, 8, 16, 100, 255, 256, 1000])
        if rng.random() < 0.1:
            k = boundary_values(rng, kind)
        limit = boundary_values(rng, kind)
        if rng.random() < 0.5:
            # Bias toward short terminating trajectories just below limit.
            span = rng.choice([0, 1, 2, 3, 5, 17, 100, 1000, 3999])
            limit = wrap(init + k + span * step + rng.choice([0, 1, -1]), kind)
        cmp = rng.choice(list(CMPS))
        cont = rng.random() < 0.7
        brk = None
        if rng.random() < 0.2:
            brk = wrap(init + step * rng.randint(0, 20), kind)
        res = simulate(kind, init, step, k, limit, cmp, cont, brk, wrapv)
        if res is None:
            continue
        T = ctype(kind)
        swap = rng.random() < 0.5
        expr = f'({T})(i + {lit(k, kind)})'
        lim = 'lim' if runtime else lit(limit, kind)
        if swap:
            flip = {'<': '>', '<=': '>=', '>': '<', '>=': '<=', '!=': '!='}[cmp]
            cond = f'{lim} {flip} {expr}'
        else:
            cond = f'{expr} {cmp} {lim}'
        if not cont:
            cond = f'!({cond})'
        start = 'ini' if runtime else lit(init, kind)
        pre = ''
        if runtime:
            pre = (f'    {T} ini = *(volatile {T} *)&g_ini_{idx};\n'
                   f'    {T} lim = *(volatile {T} *)&g_lim_{idx};\n')
        body_brk = f'        if (i == {lit(brk, kind)}) break;\n' if brk is not None else ''
        glob = ''
        if runtime:
            glob = (f'static {T} g_ini_{idx} = {lit(init, kind)};\n'
                    f'static {T} g_lim_{idx} = {lit(limit, kind)};\n')
        fn = (f'{glob}__attribute__((noinline)) static void t{idx}(void) {{\n'
              f'    unsigned long acc = 0, cnt = 0;\n{pre}'
              f'    {T} i;\n'
              f'    for (i = {start}; {cond}; i += {lit(step, kind)}) {{\n{body_brk}'
              f'        acc = acc * 31 + (unsigned long)(unsigned)i;\n'
              f'        cnt++;\n'
              f'    }}\n'
              f'    printf("t{idx} %lu %lu %lu\\n", cnt, acc, (unsigned long)i);\n}}\n')
        return fn
    return None


def gen_bytecnt(rng, idx):
    counter = rng.choice(['unsigned char', 'unsigned short', 'unsigned', 'unsigned long',
                          'int', 'long', 'unsigned long'])
    elem = rng.choice(['unsigned char', 'signed char', 'char', 'unsigned char'])
    n = rng.choice([0, 1, 2, 15, 16, 17, 31, 32, 33, 63, 64, 65, 255, 256, 257, 300, 1000, 1023, 4097])
    start = rng.choice([0, 1, 2, 3, 7, 15, 16, 17, 31, 33])
    start = min(start, n)
    end = rng.choice([n, n, max(start, n - 1), max(start, n - 3)])
    c = rng.choice([0, 1, 10, 32, 97, 127, 128, 200, 255])
    pred = rng.choice(['==', '!=', '<', '>', '<=', '>='])
    form = rng.choice(['add', 'if', 'ternary'])
    lit_c = f'({elem}){c}'
    test = f'p[i] {pred} {lit_c}'
    stmt = {'add': f'cnt += ({test});', 'if': f'if ({test}) cnt++;',
            'ternary': f'cnt += ({test}) ? 1 : 0;'}[form]
    seed = rng.randint(0, 1 << 30)
    itype = rng.choice(['unsigned long', 'long', 'unsigned', 'int'])
    return (f'__attribute__((noinline)) static {counter} bc{idx}(const {elem} *p, {itype} s, {itype} e) {{\n'
            f'    {counter} cnt = 0;\n'
            f'    for ({itype} i = s; i < e; i++) {stmt}\n'
            f'    return cnt;\n}}\n'
            f'static void t{idx}(void) {{\n'
            f'    static {elem} buf[{max(n, 1) + 64}];\n'
            f'    unsigned x = {seed}u;\n'
            f'    for (int j = 0; j < (int)sizeof buf; j++) {{ x = x * 1103515245u + 12345u;\n'
            f'        buf[j] = (x >> 16) % 3 == 0 ? {lit_c} : ({elem})(x >> 9); }}\n'
            f'    volatile {itype} vs = {start}, ve = {end};\n'
            f'    for (int off = 0; off < 3; off++)\n'
            f'        printf("t{idx} %d %lu\\n", off, (unsigned long)bc{idx}(buf + off, vs, ve));\n}}\n')


SPECIAL_D = ['0.0', '-0.0', '1.0', '-1.0', '0.5', '3.0', '1e308', '-1e308', '4.9e-324', '-4.9e-324',
             '2.2250738585072014e-308', '__builtin_inf()', '-__builtin_inf()', '__builtin_nan("")',
             '1e-300', '123456.789', '-0.1']


def gen_recip(rng, idx):
    n = rng.choice([1, 3, 4, 5, 7, 8, 9, 16, 17, 31, 64, 100])
    io = rng.choice([0, 1, 2, 7, 100, 1000, 30000])
    shape = rng.choice(['spectral', 'spectral_t', 'recip_sum', 'scaled'])
    if shape == 'spectral':
        den = '((i + j) * (i + j + 1) / 2 + i + 1)'
    elif shape == 'spectral_t':
        den = '((j + i) * (j + i + 1) / 2 + j + 1)'
    elif shape == 'recip_sum':
        den = '(i + j + 1)'
    else:
        den = '(2 * i + 3 * j + 1)'
    vals = ', '.join(rng.choice(SPECIAL_D) if rng.random() < 0.3 else repr(rng.uniform(-4, 4))
                     for _ in range(n))
    form = rng.choice(['div', 'mulrecip'])
    term = f'u[j] / {den}' if form == 'div' else f'u[j] * (1.0 / {den})'
    return (f'__attribute__((noinline)) static double rc{idx}(const double *u, int n, int i) {{\n'
            f'    double s = 0.0;\n'
            f'    for (int j = 0; j < n; j++) s += {term};\n'
            f'    return s;\n}}\n'
            f'static void t{idx}(void) {{\n'
            f'    static const double u[] = {{{vals}}};\n'
            f'    volatile int vn = {n};\n'
            f'    for (int i = {io}; i < {io} + 3; i++) {{\n'
            f'        double r = rc{idx}(u, vn, i); unsigned long b; __builtin_memcpy(&b, &r, 8);\n'
            f'        if (r != r) b = 0x7ff8000000000000ul;\n'
            f'        printf("t{idx} %d %016lx\\n", i, b); }}\n}}\n')


def small_expr(rng, vars_, depth=0):
    """Range-bounded int expression: |value| < 2^28 given |vars| < 2^12."""
    if depth > 2 or rng.random() < 0.3:
        return rng.choice(vars_ + [str(rng.randint(-300, 300))])
    a = small_expr(rng, vars_, depth + 1)
    b = small_expr(rng, vars_, depth + 1)
    op = rng.choice(['+', '-', '*', '&', '|', '^', '>>', '-'])
    if op == '*':
        return f'(({a}) % 1024 * (({b}) % 1024))'
    if op == '>>':
        return f'(({a}) >> {rng.randint(0, 5)})'
    return f'(({a}) {op} ({b}))'


def gen_sext(rng, idx):
    vars_ = ['a', 'b', 'c']
    e1 = small_expr(rng, vars_)
    e2 = small_expr(rng, vars_)
    mode = rng.choice(['index', 'long', 'rmw', 'phi', 'call'])
    if mode == 'index':
        body = (f'    int k = ({e1}) % 2000;\n'
                f'    return base[k] + (long)k;\n')
    elif mode == 'long':
        body = (f'    int k = {e1};\n    long w = k;\n    unsigned long z = (unsigned)k;\n'
                f'    return w * 3 + (long)(z >> 7) + (long)(int)({e2});\n')
    elif mode == 'rmw':
        body = (f'    int k = a; k += ({e1}) % 2000; k -= ({e2}) % 2000;\n'
                f'    int *q = &slot; *q = k; *q += b; *q -= c;\n'
                f'    long w = *q;\n    return w + base[(*q) % 2000];\n')
    elif mode == 'phi':
        body = (f'    int k;\n    if (a & 1) k = ({e1}) % 2000; else k = -(({e2}) % 2000) - 1;\n'
                f'    long w = k;\n    return base[k] * 2 + w;\n')
    else:
        body = (f'    int k = id{idx}(({e1}) % 2000);\n'
                f'    return base[k] + (long)k * 5;\n')
    rows = []
    for _ in range(6):
        rows.append((rng.randint(-4000, 4000), rng.randint(-4000, 4000), rng.randint(-4000, 4000)))
    calls = ''.join(f'    printf("t{idx} %ld\\n", sx{idx}(mid, {a}, {b}, {c}));\n' for a, b, c in rows)
    return (f'static int slot;\n'
            f'__attribute__((noinline)) static int id{idx}(int x) {{ return x; }}\n'
            f'__attribute__((noinline)) static long sx{idx}(const long *base, int a, int b, int c) {{\n'
            f'{body}}}\n'
            f'static void t{idx}(void) {{\n'
            f'    static long arr[4001];\n'
            f'    for (int j = 0; j < 4001; j++) arr[j] = (long)j * 7919 - 13;\n'
            f'    const long *mid = arr + 2000;\n{calls}}}\n')


FAMILIES = ['bound', 'runtime', 'bytecnt', 'recip', 'sext']


def generate(seed, wrapv, per_tu=48):
    rng = random.Random(seed * 2 + (1 if wrapv else 0))
    fns = []
    for idx in range(per_tu):
        fam = rng.choice(FAMILIES + ['bound', 'runtime'])
        if fam == 'bound':
            f = gen_bound(rng, idx, wrapv, False)
        elif fam == 'runtime':
            f = gen_bound(rng, idx, wrapv, True)
        elif fam == 'bytecnt':
            f = gen_bytecnt(rng, idx)
        elif fam == 'recip':
            f = gen_recip(rng, idx)
        else:
            f = gen_sext(rng, idx)
        if f is None:
            f = f'static void t{idx}(void) {{ printf("t{idx} skip\\n"); }}\n'
        fns.append(f)
    main = 'int main(void) {\n' + ''.join(f'    t{i}();\n' for i in range(per_tu)) + '    return 0;\n}\n'
    return '#include <stdio.h>\n' + ''.join(fns) + main


def run(cmd, timeout=120):
    return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--lccc', required=True)
    ap.add_argument('--gcc', default='gcc-16')
    ap.add_argument('--seeds', default='0:20')
    ap.add_argument('--levels', default='O1,O2,O3')
    ap.add_argument('--march', default='x86-64,x86-64-v3')
    ap.add_argument('--out', type=Path, default=Path('/tmp/boundary-fuzz'))
    ap.add_argument('--json', type=Path)
    ap.add_argument('--keep', action='store_true')
    ap.add_argument('--emit-only', action='store_true', help='write TUs and exit')
    args = ap.parse_args()
    lo, hi = (int(x) for x in args.seeds.split(':'))
    args.out.mkdir(parents=True, exist_ok=True)
    report = dict(lccc=args.lccc, gcc=args.gcc, seeds=args.seeds, levels=args.levels,
                  march=args.march, cases=0, mismatches=[], generator_defects=[], builds=0)
    status = 0
    for seed in range(lo, hi):
        for wrapv in (False, True):
            src = args.out / f's{seed}{"w" if wrapv else ""}.c'
            src.write_text(generate(seed, wrapv))
            if args.emit_only:
                continue
            # FP contraction is legal under GCC's default -ffp-contract=fast,
            # but which mul/add pairs fuse is a tuning choice (GCC declines
            # FMA on loop-carried reduction chains), so bit-exact comparison
            # requires the strict IEEE contract on every compiler.
            fl = ['-ffp-contract=off'] + (['-fwrapv'] if wrapv else [])
            refs = {}
            for lvl in ('-O0', '-O2'):
                exe = src.with_suffix(f'.gcc{lvl}')
                b = run([args.gcc, lvl, *fl, '-w', str(src), '-o', str(exe)])
                if b.returncode:
                    report['generator_defects'].append(dict(src=str(src), stage='gcc build', err=b.stderr[-2000:]))
                    status = 2
                    break
                refs[lvl] = run([str(exe)]).stdout
            else:
                if refs['-O0'] != refs['-O2']:
                    report['generator_defects'].append(dict(src=str(src), stage='reference disagreement'))
                    status = max(status, 2)
                    continue
                expected = refs['-O0'].splitlines()
                report['cases'] += sum(1 for line in expected if not line.endswith('skip'))
                for lvl in args.levels.split(','):
                    for march in args.march.split(','):
                        exe = src.with_suffix(f'.lccc-{lvl}-{march}')
                        cmd = [args.lccc, f'-{lvl}', f'-march={march}', *fl, '-w', str(src), '-o', str(exe)]
                        b = run(cmd, timeout=300)
                        report['builds'] += 1
                        if b.returncode:
                            report['mismatches'].append(dict(src=str(src), cmd=cmd, kind='lccc build failure',
                                                             err=b.stderr[-2000:]))
                            status = max(status, 1)
                            continue
                        r = run([str(exe)])
                        got = r.stdout.splitlines()
                        if got != expected or r.returncode:
                            first = next((i for i, (g, e) in enumerate(zip(got, expected)) if g != e),
                                         min(len(got), len(expected)))
                            report['mismatches'].append(dict(
                                src=str(src), cmd=cmd, returncode=r.returncode,
                                first_diff=dict(got=got[first] if first < len(got) else None,
                                                want=expected[first] if first < len(expected) else None)))
                            status = max(status, 1)
                        if not args.keep:
                            exe.unlink(missing_ok=True)
            print(f'seed {seed}{" -fwrapv" if wrapv else ""}: cases={report["cases"]} '
                  f'mismatches={len(report["mismatches"])} defects={len(report["generator_defects"])}', flush=True)
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(report, indent=1) + '\n')
    for m in report['mismatches'][:20]:
        print('MISMATCH', json.dumps(m)[:600])
    for d in report['generator_defects'][:5]:
        print('GENERATOR-DEFECT', json.dumps(d)[:400])
    print(('PASS' if status == 0 else 'FAIL') +
          f': {report["cases"]} boundary cases x {report["builds"]} LCCC builds; '
          f'{len(report["mismatches"])} mismatches, {len(report["generator_defects"])} generator defects')
    return status


if __name__ == '__main__':
    sys.exit(main())
