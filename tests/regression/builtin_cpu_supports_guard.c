/* AUD-CPU-1: the guarded probe initialiser must be transparent.
 *
 * Every function that reads a probe word gets one initialiser for the whole
 * function, reached through an inline test of the ready flag (see
 * `splice_cpu_probe_init`).  That makes the initialiser a call inside an
 * otherwise leaf function, so the value it must not break is *anything live
 * across it*: parameters still needed afterwards, values computed before the
 * probe, and the caller's registers in general.  The cases below are the ones
 * a register allocator gets wrong when a call is inserted late:
 *
 *   loop_probe     probe in a loop body (guard must dominate the back edge)
 *   cond_probe     probe inside a nested conditional (guard in the entry
 *                  block, not in the branch it is used from)
 *   label_probe    goto loop (a back edge, so the guard's block has several
 *                  successors and the probe is not in straight-line code)
 *   two_sites      two probe sites in one function: exactly one initialiser
 *   first_stmt     the probe is the function's first statement
 *   many_args      8 arguments, all live across the initialiser call — the
 *                  ones the ABI passes on the stack must survive it
 *   lead/recurse   a function that is otherwise inlinable/leaf, and recursion
 *
 * The expected answers are computed from `main`'s own probes, so the test is
 * host-independent: it fails only if a probe *inside* a function disagrees
 * with a probe at top level, which is exactly the failure mode of a misplaced
 * or missing initialiser.  run_regression.py additionally compares this
 * program's output byte-for-byte against GCC's.
 */
#include <stdio.h>

__attribute__((noinline)) static int loop_probe(void) {
    int i, s = 0;
    for (i = 0; i < 1000; i++) {
        s += __builtin_cpu_supports("avx2") ? 1 : 0;
    }
    return s;
}

__attribute__((noinline)) static int cond_probe(int x, int y) {
    int r = x;
    if (x) {
        if (__builtin_cpu_supports("sse2")) {
            r += y;
        } else {
            r -= y;
        }
    }
    return r;
}

__attribute__((noinline)) static int label_probe(int n) {
    int s = 0;
again:
    if (__builtin_cpu_supports("sse2")) {
        s += n;
    }
    if (n--) {
        goto again;
    }
    return s;
}

__attribute__((noinline)) static int two_sites(int a, int b) {
    int r = 0;
    if (__builtin_cpu_supports("avx2")) {
        r += a;
    }
    if (__builtin_cpu_supports("bmi2")) {
        r += b;
    }
    return r;
}

__attribute__((noinline)) static int first_stmt(void) {
    return __builtin_cpu_supports("avx512f") ? 7 : 3;
}

__attribute__((noinline)) static int many_args(int a, int b, int c, int d, int e, int f, int g,
                                               int h) {
    int s = a + b + c + d + e + f + g + h; /* live across the initialiser call */
    int p = __builtin_cpu_supports("avx2") ? 1 : 0;
    return s * 2 + p;
}

static int lead(void) { return __builtin_cpu_supports("sse2") ? 1 : 0; }

__attribute__((noinline)) static int recurse(int n) {
    if (n <= 0) {
        return __builtin_cpu_supports("avx2") ? 1 : 0;
    }
    return recurse(n - 1) + (__builtin_cpu_supports("bmi") ? 0 : 0);
}

int main(void) {
    int avx2 = __builtin_cpu_supports("avx2") ? 1 : 0;
    int sse2 = __builtin_cpu_supports("sse2") ? 1 : 0;
    int bmi2 = __builtin_cpu_supports("bmi2") ? 1 : 0;
    int avx512f = __builtin_cpu_supports("avx512f") ? 1 : 0;
    int ok = 1;

    ok &= loop_probe() == 1000 * avx2;
    ok &= cond_probe(1, 5) == 1 + (sse2 ? 5 : -5);
    ok &= cond_probe(0, 5) == 0;
    ok &= label_probe(9) == (9 + 8 + 7 + 6 + 5 + 4 + 3 + 2 + 1 + 0) * sse2;
    ok &= two_sites(3, 4) == (avx2 ? 3 : 0) + (bmi2 ? 4 : 0);
    ok &= first_stmt() == (avx512f ? 7 : 3);
    ok &= many_args(1, 2, 3, 4, 5, 6, 7, 8) == 36 * 2 + avx2;
    ok &= lead() == sse2;
    ok &= recurse(4) == avx2;

    printf("%d %d\n", ok, sse2);
    return ok ? 0 : 1;
}
