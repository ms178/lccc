#!/usr/bin/env python3
"""Differential witness for the exit-compare bound folds under -fwrapv.

Generates counted loops `for (T i = S; i + C <op> N; i += K)` whose constants
sit next to the type boundaries (INT_MIN/INT_MAX, 2^32-1, steps up to 2^30-1)
and compares every loop's observable trajectory hash with a reference
compiler at -O0 -fwrapv. Signed loops are filtered to those that terminate
under wrapping semantics (simulated here) and carry a second test of the same
IV in the body, so a range certified over a wrapping trajectory changes the
output. Unsigned loops carry a trip cap.

The run also requires that the folds actually fire (CCC_DEBUG_AFFINE_FOLD),
so a matcher change that silently stops folding cannot pass vacuously.
Mutation check (2026-10-16): disabling the three no-wrap range obligations in
`loop_rotate::loop_exit_threshold` makes seed 3 fail at -O2 and -O3.
"""
import argparse
import os
import random
import subprocess
import sys
import tempfile
from pathlib import Path

OPS = {'<': lambda a, b: a < b, '<=': lambda a, b: a <= b,
       '>': lambda a, b: a > b, '>=': lambda a, b: a >= b}
SIGNED = [0, 1, 2, 3, 4, 7, 8, 16, 100, 399, (1 << 30) - 1, 1 << 30, (1 << 31) - 1,
          (1 << 31) - 2, (1 << 31) - 8, (1 << 31) - 100, -(1 << 31), -(1 << 31) + 3, -5, -100]
UNSIGNED = [0, 1, 2, 7, 100, 1 << 20, 1 << 31, (1 << 32) - 1, (1 << 32) - 2, (1 << 32) - 8,
            (1 << 31) - 1, 1 << 30]


def w32(x):
    x &= 0xffffffff
    return x - (1 << 32) if x >= 1 << 31 else x


def lit(x):
    return f'({x})' if x > -(1 << 31) else '(-2147483647-1)'


def program(seed, count=150):
    rng = random.Random(seed)
    fns, calls = [], []
    while len(fns) < count:
        k = len(fns)
        if rng.random() < 0.6:
            s, c, n = (w32(rng.choice(SIGNED) + rng.choice([0, 0, 1, -1, 2])) for _ in range(3))
            op = rng.choice(list(OPS))
            step = rng.choice([1, 1, 2, 3, 4, (1 << 30) - 1])
            c2, n2 = (w32(rng.choice(SIGNED) + rng.choice([0, 1, -1])) for _ in range(2))
            if rng.random() < 0.4:
                # Crossing family: the controlling sum steps over INT_MAX
                # within a few iterations while the IV itself climbs close to
                # INT_MAX, so a small offset in a second test wraps -- the
                # case the no-wrap certificate exists to refuse.
                step = rng.choice([(1 << 30) - 1, (1 << 30) - 1, 1 << 29, 1000003])
                s = rng.randrange(0, 2 if step == (1 << 30) - 1 else 6)
                n = (1 << 31) - 1 - rng.randrange(0, 8)
                c = w32(n - s - step * rng.randrange(0, 3) + rng.randrange(-8, 9))
                op = rng.choice(['<', '<='])
                c2 = rng.choice([1, 2, 7, 100])
                n2 = rng.choice([0, 1000, (1 << 31) - 2])
            if op[0] == '>':
                step = -step
            i, trips = s, 0
            while OPS[op](w32(i + c), n) and trips <= 600:
                i, trips = w32(i + step), trips + 1
            if trips > 600:
                continue
            head = f"for (int i = {lit(s)}; i + {lit(c)} {op} {lit(n)}; i += {lit(step)})"
            # A second test of the same IV borrows the controlling range: if
            # that range is certified over a wrapping trajectory, it folds
            # wrongly (the #841 `affine_control_proof.c` class).
            op2 = rng.choice(list(OPS))
            # Half the loops keep `i` out of the hash, so the IV stays a
            # plain 32-bit counter instead of being widened for the cast.
            seen = "(unsigned long long)(long long)i" if k % 2 else "(unsigned long long)++trips"
            body = (f"h = h * 31 + {seen} + (unsigned)*g; "
                    f"h += (i + {lit(c2)} {op2} {lit(n2)});")
        else:
            s, c, n = ((rng.choice(UNSIGNED) + rng.choice([0, 0, 1, -1, 3])) % (1 << 32)
                       for _ in range(3))
            op = rng.choice(['<', '<='])
            step = rng.choice([1, 2, 3, 7, 1 << 20, 1 << 31])
            head = f"for (unsigned i = {s}u; (unsigned)(i + {c}u) {op} {n}u; i += {step}u)"
            body = ("h = h * 31 + i + (unsigned)*g; if (++trips == 600) break;")
        fns.append(f"__attribute__((noinline)) static unsigned long long t{k}(volatile int *g) "
                   f"{{ unsigned long long h = 0; int trips = 0; (void)trips; {head} {{ {body} }} "
                   f"return h; }}")
        calls.append(f'printf("%d %llu\\n", {k}, t{k}(&g));')
    return ("#include <stdio.h>\nvolatile int g = 0;\n" + "\n".join(fns) +
            "\nint main(void) {\n" + "\n".join(calls) + "\nreturn 0;\n}\n")


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, timeout=120, **kw)


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--lccc', required=True)
    ap.add_argument('--cc', default=os.environ.get('GCC_BIN', 'gcc'))
    ap.add_argument('--seeds', type=int, default=4)
    ap.add_argument('--min-folds', type=int, default=20)
    args = ap.parse_args()
    opts = [['-O1'], ['-O2'], ['-O3', '-march=x86-64-v3']]
    failures, folds = [], 0
    with tempfile.TemporaryDirectory(prefix='affine-wrapv-') as tmp:
        tmp = Path(tmp)
        for seed in range(1, args.seeds + 1):
            src = tmp / f'seed{seed}.c'
            src.write_text(program(seed))
            ref = tmp / 'ref'
            p = run([args.cc, '-w', '-O0', '-fwrapv', str(src), '-o', str(ref)])
            if p.returncode:
                print(f'FAIL: reference compile seed {seed}: {p.stderr.decode()[-400:]}')
                return 1
            expected = run([str(ref)]).stdout
            for opt in opts:
                exe = tmp / 'lccc.out'
                p = run([args.lccc, '-w', '-fwrapv', *opt, str(src), '-o', str(exe)])
                got = run([str(exe)]).stdout if p.returncode == 0 else None
                if got != expected:
                    failures.append((seed, ' '.join(opt), p.stderr.decode()[-200:]))
            env = dict(os.environ, CCC_DEBUG_AFFINE_FOLD='1')
            p = run([args.lccc, '-w', '-fwrapv', '-O2', '-c', str(src), '-o', os.devnull], env=env)
            folds += sum(line.startswith('[AFFOLD]') for line in p.stderr.decode().splitlines())
    for seed, opt, err in failures:
        print(f'FAIL: seed {seed} {opt}: trajectory differs from the -O0 -fwrapv reference {err}')
    if folds < args.min_folds:
        print(f'FAIL: only {folds} exit-compare folds fired (< {args.min_folds}); the witness is vacuous')
        return 1
    if failures:
        return 1
    print(f'PASS: affine -fwrapv differential: {args.seeds} seeds x 150 loops x {len(opts)} levels, '
          f'{folds} folds fired, 0 mismatches')
    return 0


if __name__ == '__main__':
    sys.exit(main())
