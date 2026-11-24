/* Loop rotation: what the loop test computes, read outside the test.
 *
 * Rotation leaves the header behind as a guard that runs once and puts a
 * copy of the test at the end of the body. A value the header computes for
 * the test then has two definitions, the guard's and the copy's, and a
 * reader in the body or behind the loop needs the one of the iteration it
 * is in. The pass merged the two for the header's phis only; every other
 * value kept the guard's for the whole loop:
 *
 *   for (i = 2; i * i < n; i++) t += i * i;            added 4 each time
 *   for (i = 0; (t = i + 1) < n; i++) s += a[t];       summed a[1] n-1 times
 *   for (i = 0; (t = i * 3) < n; i++) ...; return t;   returned 0
 *
 * The first needs no assignment in the test: value numbering gives the
 * body's `i * i` the header's name (gvn_cross_block_mul_dot_product.c is
 * that loop; under rotation both its function and the loop it checks the
 * function against computed 162 for 1597, and agreed).
 *
 * With CCC_LOOP_ROTATE=1 at -O2 and -O3. GCC 16.2 and Clang 23.1.5 agree on
 * every line this program prints, and the address and undefined-behaviour
 * sanitizers have nothing to say about it. lccc printed:
 *
 *   squares(b, 300)   532              for 2252     (4 each time)
 *   body_use(a, 10)   -2016            for -684     (a[1] nine times)
 *   after_use(10)     0                for 12
 *   narrow(a, 10)     -4479            for -1139
 *   twice(a, 10)      -2795            for -1315
 *   down(a, 10)       114792           for 54268
 *   fill(out, 30)     0 0 0 0 0 0 0    for 0 1 4 9 16 25 36
 *
 * The widening of a narrow counter is such a value as well, as soon as the
 * body uses the header's instead of its own (value numbering through casts,
 * CCC_CAST_RENAME=1): then `unsigned char i; ... a[i + 1]` read a[1] on
 * every iteration (u8off(a, 10) was -2240 for -575), which is how this was
 * found. The companion tests compile this file with both knobs.
 *
 * The loops are refused now (loop_rotate.rs, Guard H). The neighbours that
 * read only the counter are still rotated and are held here to the same
 * results, so that the refusal is not bought with theirs.
 */
#include <stdio.h>

long g;

/* No assignment in the test: the body's `i * i` is the header's. */
__attribute__((noinline)) static long squares(const long *b, long n) {
    long s = 0;
    long t = 0;
    for (long i = 2; i * i < n; i++) {
        s += b[i];
        t += i * i;
    }
    return s * 3 + t;
}

/* The test's value, read in the body. */
__attribute__((noinline)) static long body_use(const int *a, int n) {
    long s = 0;
    int i, t;
    for (i = 0; (t = i + 1) < n; i++) {
        s += a[t];
    }
    return s;
}

/* The test's value, read behind the loop. */
__attribute__((noinline)) static int after_use(int n) {
    int i, t;
    for (i = 0; (t = i * 3) < n; i++) {
        g += i;
    }
    return t;
}

/* Both, with a narrow counter. */
__attribute__((noinline)) static long narrow(const int *a, unsigned char n) {
    long s = 0;
    unsigned char i;
    int t;
    for (i = 0; (t = i + 1) <= n; i++) {
        s += a[t] * 2;
    }
    return s + t;
}

/* Two values of the test, one computed from the other. */
__attribute__((noinline)) static long twice(const int *a, int n) {
    long s = 0;
    int i, t, u;
    for (i = 0; (t = i * 2, u = t + 1) < n; i++) {
        s += a[t] * 3 - a[u];
    }
    return s;
}

/* The test's value written through a pointer in the body. */
__attribute__((noinline)) static void fill(int *out, int n) {
    int i, t;
    for (i = 0; (t = i * i) < n; i++) {
        out[i] = t;
    }
}

/* A downward loop whose test value is read in the body and behind it. */
__attribute__((noinline)) static unsigned long down(const int *a, int n) {
    unsigned long s = 0;
    int i, t;
    for (i = n; (t = i - 2) > 0; i--) {
        s = s * 3 + (unsigned long)a[t];
    }
    return s - (unsigned long)t;
}

/* The neighbours: only the counter is read. These rotate. */
__attribute__((noinline)) static long plain(const int *a, int n) {
    long s = 0;
    for (int i = 0; i < n; i++) {
        s += a[i + 1];
    }
    return s;
}

__attribute__((noinline)) static long plain_after(int n) {
    int i;
    for (i = 0; i * 3 < n; i++) {
        g += i;
    }
    return i;
}

/* The narrow counter indexed past itself: with value numbering through
 * casts, the body's `(int)i` is the header's. */
__attribute__((noinline)) static long u8off(const int *a, unsigned char n) {
    long s = 0;
    unsigned char i;
    for (i = 0; i < n; i++) {
        s += a[i + 1];
    }
    return s;
}

__attribute__((noinline)) static long s16off(const int *a, short n) {
    long s = 0;
    short i;
    for (i = 0; i < n; i++) {
        s += a[i + 2] - a[i];
    }
    return s;
}

int main(void) {
    static int a[700];
    static long b[64];
    static int out[64];
    for (int i = 0; i < 700; i++) {
        a[i] = (i * 37 - 11) % 501 - 250;
    }
    for (int i = 0; i < 64; i++) {
        b[i] = (i * 53 + 7) % 97 - 40;
    }
    unsigned long h = 0;
    for (int n = 0; n < 300; n++) {
        h = h * 31 + (unsigned long)squares(b, n);
        h = h * 31 + (unsigned long)body_use(a, n);
        h = h * 31 + (unsigned long)after_use(n);
        h = h * 31 + (unsigned long)narrow(a, (unsigned char)(n & 0xff));
        h = h * 31 + (unsigned long)twice(a, n);
        h = h * 31 + down(a, n);
        h = h * 31 + (unsigned long)plain(a, n);
        h = h * 31 + (unsigned long)plain_after(n);
        h = h * 31 + (unsigned long)u8off(a, (unsigned char)(n & 0xff));
        h = h * 31 + (unsigned long)s16off(a, (short)n);
    }
    for (int n = 0; n < 40; n++) {
        for (int i = 0; i < 64; i++) {
            out[i] = -1;
        }
        fill(out, n);
        for (int i = 0; i < 64; i++) {
            h = h * 31 + (unsigned long)out[i];
        }
    }
    printf("%lu %ld\n", h, g);
    /* The lines the defect changed, by themselves. */
    printf("squares %ld\n", squares(b, 300));
    printf("body_use %ld\n", body_use(a, 10));
    printf("after_use %d\n", after_use(10));
    printf("narrow %ld\n", narrow(a, 10));
    printf("twice %ld\n", twice(a, 10));
    printf("down %lu\n", down(a, 10));
    printf("plain %ld %ld\n", plain(a, 10), plain_after(10));
    printf("u8off %ld %ld\n", u8off(a, 10), u8off(a, 250));
    printf("s16off %ld\n", s16off(a, 10));
    fill(out, 30);
    printf("fill %d %d %d %d %d %d %d\n", out[0], out[1], out[2], out[3], out[4], out[5], out[6]);
    return 0;
}
