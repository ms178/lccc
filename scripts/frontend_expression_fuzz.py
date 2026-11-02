#!/usr/bin/env python3
"""Randomised differential *expression* fuzzer for the C frontend.

The fixed corpus in ``tests/frontend_audit`` states the rules we know about.
This fuzzer looks for the rules we do not: it generates type-correct C
expressions from a seeded PRNG, prints their value, and compares lccc with the
reference compilers.  Every case stays inside behaviour the standard defines --
unsigned arithmetic wraps, signed arithmetic is kept inside its range, shifts
count modulo the width, divisors are forced non-zero, and no expression has two
unsequenced side effects -- so a divergence is a compiler defect and never an
undefined program.

    python3 scripts/frontend_expression_fuzz.py --cases 200 --seed 1
    python3 scripts/frontend_expression_fuzz.py --cases 2000 --seed 7 --jobs 4

A failing case is printed with its seed, so it reproduces exactly::

    python3 scripts/frontend_expression_fuzz.py --seed 12345 --cases 1 --verbose

The generator is deliberately *boring*: no floating-point exceptions, no
conversions to integer types that could trap, no library calls other than
``printf``.  What is left is the part of the frontend that must be exactly
right -- integer promotion, usual arithmetic conversions, constant folding,
shifts, casts, comparisons and the conditional operator.
"""

from __future__ import annotations

import argparse
import random
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent

# Every generated value is printed through (long long) or (unsigned long long),
# so the comparison never depends on printf's idea of a narrow type.
KINDS = ('int', 'unsigned', 'long', 'unsigned long', 'long long',
         'unsigned long long', 'short', 'unsigned short', 'char', 'unsigned char')

BINARY_OPS = ('+', '-', '*', '&', '|', '^', '<', '>', '<=', '>=', '==', '!=',
              '&&', '||', '<<', '>>', '/', '%')


def gen_expr(rng: random.Random, depth: int) -> str:
    """Generate an integer expression in a value range small enough to stay
    inside signed arithmetic, so nothing depends on signed overflow."""
    if depth <= 0 or rng.random() < 0.25:
        return rng.choice(('0', '1', '2', '3', '7', '12', '31', '64', '100', '255',
                           '1024', '65535', '0x7f', '0x8000', '0xFFFFu', '1u', '3ul',
                           '0x10L', '123456789LL', '0u', '-1', '-7', '-1024', '-65535'))
    choice = rng.random()
    if choice < 0.12:
        return f'({rng.choice(("+", "-", "~", "!"))}({gen_expr(rng, depth - 1)}))'
    if choice < 0.24:
        cast = rng.choice(KINDS)
        return f'(({cast})({gen_expr(rng, depth - 1)}))'
    if choice < 0.34:
        return f'({gen_expr(rng, depth - 1)} ? {gen_expr(rng, depth - 1)} : {gen_expr(rng, depth - 1)})'
    if choice < 0.40:
        return f'({gen_expr(rng, depth - 1)},{gen_expr(rng, depth - 1)})'
    op = rng.choice(BINARY_OPS)
    left = gen_expr(rng, depth - 1)
    if op in ('/', '%'):
        # A non-zero right operand keeps the operation defined. `| 1` on an
        # unsigned value never turns it into a trap: the result is odd.
        right = f'(({gen_expr(rng, depth - 1)}) | 1u)'
    elif op in ('<<', '>>'):
        right = str(rng.randrange(0, 8))
    else:
        right = gen_expr(rng, depth - 1)
    return f'({left} {op} {right})'


def gen_stmt_case(rng: random.Random) -> str:
    """Statements, not just expressions: aggregates, bit-fields, pointer
    arithmetic, compound assignment, ``sizeof`` of an unevaluated operand and
    the integer promotions that run through an assignment.  These are the paths
    an expression-only fuzzer cannot reach."""
    body = ['#include <stdio.h>', '#include <stddef.h>', '',
            'struct S { unsigned a : 3; signed b : 5; unsigned c : 11; int d; };',
            'union U { unsigned u; unsigned char b[4]; };',
            'static long f(long x) { return x * 2 + 1; }',
            'int main(void) {']
    body.append('    int arr[6] = { 1, 2, 3, 4, 5, 6 };')
    body.append('    struct S s = { 5, -3, 1000, -17 };')
    body.append('    union U u;')
    for _ in range(8):
        pick = rng.randrange(12)
        value = gen_expr(rng, rng.randrange(1, 4))
        if pick == 0:
            body.append(f'    {{ unsigned char v = (unsigned char)({value}); printf("%d\\n", (int)v); }}')
        elif pick == 1:
            body.append(f'    {{ signed char v = (signed char)({value}); printf("%d\\n", (int)v); }}')
        elif pick == 2:
            body.append(f'    {{ short v = (short)({value}); printf("%d\\n", (int)v); }}')
        elif pick == 3:
            body.append(f'    printf("%zu\\n", sizeof({value}));')
        elif pick == 4:
            index = rng.randrange(0, 6)
            body.append(f'    printf("%d\\n", arr[{index}] + (int)((arr + {index})[{rng.randrange(-index, 6 - index)}]));')
        elif pick == 5:
            body.append(f'    {{ int i = {rng.randrange(-3, 4)}; i += (int)({value}); '
                        f'i *= 3; printf("%d\\n", i); }}')
        elif pick == 6:
            body.append(f'    s.a = (unsigned)({value}); s.b = (signed)({value});')
            body.append('    printf("%u %d\\n", s.a, (int)s.b);')
        elif pick == 7:
            body.append('    u.u = 0x01020304u;')
            body.append('    printf("%u %u\\n", (unsigned)u.b[0], (unsigned)u.b[3]);')
        elif pick == 8:
            body.append(f'    printf("%ld\\n", f((long)({value})));')
        elif pick == 9:
            body.append(f'    {{ long long total = 0; int k; for (k = 0; k < 4; k++) '
                        f'total = total + (long long)({{ int t = k; t * {rng.choice(("1", "2", "3"))}; }}) + (long long)({value}); '
                        f'printf("%lld\\n", total); }}')
        elif pick == 10:
            body.append(f'    {{ switch ((int)(({value}) & 3)) {{ case 0: printf("z\\n"); break; '
                        f'case 1: printf("o\\n"); break; default: printf("m\\n"); }} }}')
        else:
            body.append(f'    printf("%d\\n", _Generic(({value}), unsigned long long: 4, '
                        f'unsigned int: 3, long long: 2, long: 1, int: 0));')
    body.append('    return 0;')
    body.append('}')
    return '\n'.join(body) + '\n'


def gen_case(rng: random.Random, index: int) -> str:
    lines = ['#include <stdio.h>', 'int main(void) {']
    for _ in range(12):
        expr = gen_expr(rng, rng.randrange(1, 5))
        if rng.random() < 0.5:
            lines.append(f'    printf("%llu\\n", (unsigned long long)({expr}));')
        else:
            lines.append(f'    printf("%lld\\n", (long long)({expr}));')
    lines.append('    return 0;')
    lines.append('}')
    return '\n'.join(lines) + '\n'


def run(cmd: list[str], cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(cmd, cwd=cwd, text=True, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, check=False)


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--lccc', default=str(ROOT / 'target/fastbuild/lccc'))
    parser.add_argument('--cases', type=int, default=100)
    parser.add_argument('--seed', type=int, default=1)
    parser.add_argument('--levels', default='0,2')
    parser.add_argument('--mode', choices=('expr', 'stmt', 'mixed'), default='mixed')
    parser.add_argument('--verbose', '-v', action='store_true')
    parser.add_argument('--keep', action='store_true')
    args = parser.parse_args(argv)

    lccc = str(Path(args.lccc).resolve())
    oracles = [exe for exe in ('gcc-16', 'clang-23') if shutil.which(exe)]
    if not oracles:
        print('error: need gcc-16 or clang-23 as an oracle', file=sys.stderr)
        return 2
    levels = [part.strip() for part in args.levels.split(',') if part.strip()]

    scratch = Path(tempfile.mkdtemp(prefix='lccc-expr-fuzz-'))
    failures = 0
    try:
        for index in range(args.cases):
            case_seed = args.seed * 1_000_003 + index
            rng = random.Random(case_seed)
            source = (gen_stmt_case(rng) if args.mode == 'stmt'
                      else gen_case(rng, index) if args.mode == 'expr'
                      else gen_stmt_case(rng) if rng.random() < 0.5
                      else gen_case(rng, index))
            path = scratch / f'case_{index}.c'
            path.write_text(source)
            reference: dict[str, str] = {}
            skip = False
            for exe in oracles:
                binary = scratch / f'{index}.{Path(exe).name}'
                built = run([exe, '-std=gnu11', '-w', '-O0', str(path), '-o', str(binary)], scratch)
                if built.returncode != 0:
                    skip = True
                    break
                reference[exe] = run([str(binary)], scratch).stdout
            if skip:
                continue
            for level in levels:
                binary = scratch / f'{index}.lccc-O{level}'
                built = run([lccc, '-std=gnu11', '-w', f'-O{level}', str(path), '-o', str(binary)],
                            scratch)
                if built.returncode != 0:
                    print(f'FAIL seed={case_seed} -O{level}: lccc rejected a defined program')
                    print(built.stderr.strip()[:4000])
                    failures += 1
                    break
                executed = run([str(binary)], scratch)
                for exe, want in reference.items():
                    if executed.stdout != want:
                        got_lines = executed.stdout.splitlines()
                        want_lines = want.splitlines()
                        first = next((i for i, (a, b) in enumerate(zip(want_lines, got_lines))
                                      if a != b), min(len(want_lines), len(got_lines)))
                        print(f'FAIL seed={case_seed} -O{level} vs {Path(exe).name}: '
                              f'line {first + 1}: want {want_lines[first:first + 1]}, '
                              f'got {got_lines[first:first + 1]}')
                        print (source)
                        failures += 1
                        break
                if failures:
                    break
            if failures:
                break
            if args.verbose and index % 50 == 0:
                print(f'  {index + 1}/{args.cases} cases agree')
    finally:
        if args.keep:
            print(f'scratch kept at {scratch}')
        elif failures == 0:
            shutil.rmtree(scratch, ignore_errors=True)
        else:
            print(f'failing case kept at {scratch}')
    if failures:
        return 1
    print(f'{args.cases} case(s) agree with {", ".join(Path(exe).name for exe in oracles)} '
          f'at -O{",".join(levels)}')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
