/* ZERO-ROT-AFFINE regression corpus — the canonical countdown shapes from
 * engineering/FOLLOWUP-2026-09-30-affine-exit-compare.md.
 *
 * `loop_rotate` clones the header guard into the latch, so the cloned compare
 * is `add(iv, C) < N` and the backend materialises the `+C` as a fresh
 * temporary before every compare (5 instructions per iteration against GCC's
 * 4).  `canonicalise_affine_exit_cmps` folds the constant into the bound
 * (`iv < N - C`) for signed compares only, which also gives the compare-branch
 * fusion a bare-register compare to fuse on the back edge.
 *
 * The file is compiled with CCC_LOOP_ROTATE=1 (see the sibling .env) so the
 * regression corpus exercises the rotated path against the GCC oracle, and is
 * also the fixture of tests/regression/check_affine_exit_compare.sh, which
 * pins the fold's object-code shape.
 *
 * Shapes:
 *   f_const         `i + 4 < 2048`      — the fold's canonical victim
 *   f_const_canon   `i < 2044`          — already canonical: nothing to fold
 *   sum_affine      `i + 4 < N`         — runtime bound
 *   sum_ptr         `p + 4 < v + N`     — pointer IV form of the same shape
 *   f_unsigned      unsigned `i + 4 < N`— signed-only guard: must stay correct
 *   f_bool          `go = (i + 4 < N)`  — the define-mimic chain that made the
 *                                         compare-branch fusion refuse before
 *                                         the FileLiveness backward-edge fix
 *
 * Every reference loop reads through a `volatile` pointer, so the compiler
 * cannot fold the reference the same way it folds the function under test.
 */
#include <stdio.h>

static unsigned char buf[8192];

int sum_affine(const unsigned char *v, int N) {
    int s = 0;
    for (int i = 0; i + 4 < N; i++)
        s += v[i];
    return s;
}

long sum_ptr(const unsigned char *v, int N) {
    long s = 0;
    for (const unsigned char *p = v; p + 4 < v + N; p++)
        s += *p;
    return s;
}

long f_const(const unsigned char *v) {
    long s = 0;
    for (int i = 0; i + 4 < 2048; i++)
        s += v[i];
    return s;
}

long f_const_canon(const unsigned char *v) {
    long s = 0;
    for (int i = 0; i < 2044; i++)
        s += v[i];
    return s;
}

unsigned f_unsigned(const unsigned char *v, unsigned N) {
    unsigned s = 0;
    for (unsigned i = 0; i + 4 < N; i++)
        s += v[i];
    return s;
}

int f_bool(const unsigned char *v, int N) {
    long s = 0;
    int go = 1;
    for (int i = 0; i + 4 < N; i++) {
        go = (i + 4 < N);
        s += v[i] * go;
    }
    return (int)s;
}

int main(void) {
    /* Deterministic buffer (own LCG: no libc rand() dependence). */
    unsigned x = 12345u;
    for (int i = 0; i < 8192; i++) {
        x = x * 1103515245u + 12345u;
        buf[i] = (unsigned char)(x >> 16);
    }

    for (int rep = 0; rep < 64; rep++) {
        volatile unsigned char *vp = buf;
        long a = 0, b = 0, c = 0, d = 0;
        unsigned u = 0;
        for (int i = 0; i + 4 < 2048; i++)
            a += vp[i];
        for (int i = 0; i < 2044; i++)
            b += vp[i];
        for (int i = 0; i + 4 < 2044; i++)
            c += vp[i];
        for (const unsigned char *p = buf; p + 4 < buf + 2044; p++)
            d += *p;
        for (unsigned i = 0; i + 4 < 2044u; i++)
            u += buf[i];

        if (f_const(buf) != a) {
            printf("FAIL f_const: %ld != %ld\n", f_const(buf), a);
            return 1;
        }
        if (f_const_canon(buf) != b) {
            printf("FAIL f_const_canon: %ld != %ld\n", f_const_canon(buf), b);
            return 1;
        }
        if (sum_affine(buf, 2044) != (int)c) {
            printf("FAIL sum_affine: %d != %ld\n", sum_affine(buf, 2044), c);
            return 1;
        }
        if (sum_ptr(buf, 2044) != d) {
            printf("FAIL sum_ptr: %ld != %ld\n", sum_ptr(buf, 2044), d);
            return 1;
        }
        if (f_unsigned(buf, 2044u) != u) {
            printf("FAIL f_unsigned: %u != %u\n", f_unsigned(buf, 2044u), u);
            return 1;
        }

        for (int N = 0; N <= 64; N++) {
            long r = 0;
            for (int i = 0; i + 4 < N; i++)
                r += buf[i] * (unsigned char)(i + 4 < N);
            if (f_bool(buf, N) != (int)r) {
                printf("FAIL f_bool N=%d: %d != %d\n", N, f_bool(buf, N), (int)r);
                return 1;
            }
        }
        buf[rep] ^= 0x5a; /* next round recomputes its references */
    }

    printf("affine-exit-compare: all shapes exact\n");
    return 0;
}
