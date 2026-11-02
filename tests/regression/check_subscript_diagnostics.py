#!/usr/bin/env python3
"""C11 6.5.2.1p1 subscript constraint: diagnose, never miscompile.

One operand of `E1[E2]` must have pointer type (an array decays; GNU vectors
are subscriptable) and the other integer type.  LCCC accepted every violation
and lowered it as a pointer access. Public SIMD types are now native vectors,
so their subscripts must execute correctly, not remain a rejection exception.

Gates:
  * every rejected input produces a located diagnostic (no panic/ICE),
    and GCC rejects the same input;
  * the valid forms (array, pointer, reversed `i[a]`, character, enum, bool
    and `_Bool`-typed indices, string literals, GNU vectors read and written,
    pointer-to-array, function-pointer table, compound literal) compile and
    print GCC's output -- a gate that only asserted rejections would pass
    with subscripting deleted;
  * vector-valued operands whose inferred type is the scalar element
    (vector comparison, negation and arithmetic results, casts to an
    attribute-spelled vector type, `__typeof__` of a comparison, statement
    expressions, conditionals) are NOT diagnosed: the first version of this
    check rejected the replayed reproducer
    `artifacts/repros/crash_gen_slot_stress_seed20260930_-O2.c` that way;
  * vector types survive `__typeof__` and `__auto_type`: a comparison of
    vectors is a vector of signed lanes (16 bytes here, `int` lanes for
    `float`), and `__typeof__(v)` / `__auto_type u = v - w` declare vectors.
    lccc used to type all three as scalars (sizeof 4 or 8), so `m[1]`
    segfaulted at run time.
"""
import argparse
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
NOT_INTEGER = 'array subscript is not an integer'
NOT_ADDRESSABLE = 'subscripted value is neither array nor pointer nor vector'

REJECTED = {
    'struct_base.c': ('struct S { float a[8]; };\n'
                      'float f(struct S s) { s[0] = 2.0f; return s[1]; }\n', NOT_ADDRESSABLE),
    'struct_reversed.c': ('struct S { int a; };\n'
                          'int f(struct S s) { return 0[s]; }\n', NOT_ADDRESSABLE),
    'int_base.c': ('int f(int x) { return x[0]; }\n', NOT_ADDRESSABLE),
    # Commutativity is pointer + integer only (6.5.2.1p1): a vector operand
    # cannot be the "pointer" of a reversed subscript, lvalue or call result.
    'vector_reversed.c': ('typedef int V __attribute__((vector_size(16)));\n'
                          'int f(V v) { return 1[v]; }\n', NOT_ADDRESSABLE),
    'vector_call_reversed.c': ('typedef int V __attribute__((vector_size(16)));\n'
                               'V g(void);\n'
                               'int f(void) { return 0[g()]; }\n', NOT_ADDRESSABLE),
    'double_index.c': ('int f(int *p) { return p[1.0]; }\n', NOT_INTEGER),
    'pointer_index.c': ('int f(int *p, int *q) { return p[q]; }\n', NOT_INTEGER),
    'struct_index.c': ('struct S { int a; };\n'
                       'int f(int *p, struct S s) { return p[s]; }\n', NOT_INTEGER),
    'reversed_float_base.c': ('int f(int *p, float x) { return x[p]; }\n', NOT_INTEGER),
}

VALID = r'''
#include <stdio.h>
#include <immintrin.h>
typedef int v4si __attribute__((vector_size(16)));
typedef unsigned long long v2u __attribute__((vector_size(16)));
enum E { ZERO, ONE, TWO };
static int add1(int x) { return x + 1; }
static int dbl(int x) { return x * 2; }
int main(void) {
  int a[4] = {10, 20, 30, 40}, *p = a + 1;
  int (*pa)[4] = &a;
  int (*fns[2])(int) = {add1, dbl};
  char c = 2;
  enum E e = TWO;
  _Bool b = 1;
  unsigned long long ul = 3;
  v4si v = {1, 2, 3, 4};
  v[2] = 7;
  v[b] += a[e];
  printf("%d %d %d %d %d %d %d\n", a[1], 2[a], p[-1], p[c], (*pa)[3], fns[1](a[b]), a[ul]);
  printf("%c %c %d %d %d %d\n", "lccc"[1], 2["abc"], v[0], v[1], v[2], v[3]);
  printf("%d\n", ((int[]){5, 6, 7})[ONE] + fns[0](0[p]));
  v2u x = {3, 5}, y = {7, 11};
  __typeof__(x == y) m = x == x;
  unsigned long long __attribute__((vector_size(16))) z = x * y;
  printf("%llu %llu %lld %lld %llu %llu\n",
         ((unsigned long long __attribute__((vector_size(16))))x)[1], (x + y)[0],
         (long long)(x == x)[0], (long long)m[1], (-x)[1] + z[0], (b ? x : y)[1] + ({ y; })[0]);
  __typeof__(x) t = y;
  __auto_type u = x - y;
  float __attribute__((vector_size(16))) f = {1, 2, 0.5f, -3};
  __typeof__(f < 1) fm = f < 1;
  printf("%zu %zu %zu %zu %zu %llu %llu %d %d\n", sizeof m, sizeof(__typeof__(x < y)), sizeof t,
         sizeof u, sizeof fm, t[1], u[1], fm[1], fm[3]);
  float lanes[8] = {1, 2, 3, 4, 5, 6, 7, 8};
  __m256 native = _mm256_loadu_ps(lanes);
  native[0] = 23.0f;
  native[7] += 11.0f;
  __m128 small = _mm_setzero_ps();
  small[2] = native[7];
  printf("%.0f %.0f %.0f\n", (double)native[0], (double)native[7], (double)small[2]);
  return 0;
}
'''


def run(cmd, check=True):
    result = subprocess.run(cmd, capture_output=True, text=True)
    if check and result.returncode:
        sys.exit(f'FAIL: command failed: {" ".join(map(str, cmd))}\n{result.stderr}')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default=os.environ.get('CCC', str(ROOT / 'target/fastbuild/lccc')))
    parser.add_argument('--gcc', default=os.environ.get('GCC', 'gcc'))
    args = parser.parse_args()
    with tempfile.TemporaryDirectory() as work:
        work = Path(work)
        for name, (source, message) in REJECTED.items():
            path = work / name
            path.write_text(source)
            mine = run([args.lccc, '-O2', '-mavx', '-S', path, '-o', work / 'out.s'], check=False)
            if mine.returncode == 0:
                sys.exit(f'FAIL: {name} was accepted; it must be diagnosed')
            if re.search(r'panicked at|internal compiler error|RUST_BACKTRACE', mine.stderr):
                sys.exit(f'FAIL: {name} ICEd instead of diagnosing:\n{mine.stderr}')
            if not re.search(rf'{re.escape(name)}:\d+:\d+: error: {re.escape(message)}', mine.stderr):
                sys.exit(f'FAIL: {name}: expected "{message}", got:\n{mine.stderr}')
            gcc = run([args.gcc, '-O2', '-mavx', '-S', path, '-o', work / 'gcc.s'], check=False)
            if gcc.returncode == 0:
                verdict = 'accepts' if gcc.returncode == 0 else 'rejects'
                sys.exit(f'FAIL: oracle drift: GCC now {verdict} {name}')
        valid = work / 'valid.c'
        valid.write_text(VALID)
        run([args.gcc, '-O2', '-mavx', valid, '-o', work / 'gcc'])
        reference = run([work / 'gcc']).stdout
        for opt in ('-O0', '-O2'):
            exe = work / f'lccc{opt}'
            run([args.lccc, opt, '-mavx', valid, '-o', exe])
            mine = run([exe]).stdout
            if mine != reference:
                sys.exit(f'FAIL: valid subscripts at {opt}: {mine!r} != GCC {reference!r}')
    print(f'check_subscript_diagnostics: PASS ({len(REJECTED)} invalid subscripts '
          'diagnosed (GCC agrees); valid forms including native public vectors match GCC at -O0/-O2)')


if __name__ == '__main__':
    main()
