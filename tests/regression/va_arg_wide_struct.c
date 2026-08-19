/* Regression: va_arg of long double (full 80-bit x87), __int128, structs with
 * SysV eightbyte classification, a >8-aligned struct, and va_copy.
 *
 * Before the variadic rewrite:
 *   - va_arg(ap, long double) truncated the value to f64 (fldt -> fstpl);
 *   - va_arg(ap, __int128) read only 8 bytes and advanced gp_offset by 8;
 *   - >8-aligned MEMORY structs were never 16-aligned in the overflow area.
 * Each sub-case is self-checking so the test is meaningful without a GCC
 * oracle; run_regression.sh --compare-gcc additionally diffs the output. */
#include <stdarg.h>
#include <stdio.h>
#include <string.h>

/* ---- long double: 80-bit mantissa, must survive the va_list round-trip ---- */
static int check_ld(int tag, ...) {
    long double expect = 0x1p+63L + 1.0L; /* 2^63 + 1: not representable in f64 */
    unsigned char before[10], after[10];

    va_list ap;
    va_start(ap, tag);
    long double got = va_arg(ap, long double);
    va_end(ap);

    memcpy(before, &expect, 10);
    memcpy(after, &got, 10);
    return memcmp(before, after, 10) != 0;
}

/* ---- __int128 through the register pair and the overflow paths ---- */
static __int128 isum(int n, ...) {
    va_list ap; va_start(ap, n);
    __int128 s = 0;
    for (int i = 0; i < n; i++) s += va_arg(ap, __int128);
    va_end(ap);
    return s;
}

static unsigned __int128 usum(int n, ...) {
    va_list ap; va_start(ap, n);
    unsigned __int128 s = 0;
    for (int i = 0; i < n; i++) s += va_arg(ap, unsigned __int128);
    va_end(ap);
    return s;
}

/* ---- struct eightbyte classification ---- */
struct dl { double d; long l; };         /* [Sse, Integer]  */
struct ld { long l; double d; };         /* [Integer, Sse]  */
struct dd { double a, b; };              /* [Sse, Sse]      */
struct ll { long a, b; };                /* [Integer,Integer] */
struct big { long a, b, c, d; };         /* MEMORY (32B)    */
struct a16 { int x; } __attribute__((aligned(16))); /* align 16 */

static struct dl sum_dl(int n, ...) {
    va_list ap; va_start(ap, n);
    struct dl r = {0, 0};
    for (int i = 0; i < n; i++) { struct dl t = va_arg(ap, struct dl); r.d += t.d; r.l += t.l; }
    va_end(ap);
    return r;
}
static struct ld sum_lds(int n, ...) {
    va_list ap; va_start(ap, n);
    struct ld r = {0, 0};
    for (int i = 0; i < n; i++) { struct ld t = va_arg(ap, struct ld); r.l += t.l; r.d += t.d; }
    va_end(ap);
    return r;
}
static struct dd sum_dd(int n, ...) {
    va_list ap; va_start(ap, n);
    struct dd r = {0, 0};
    for (int i = 0; i < n; i++) { struct dd t = va_arg(ap, struct dd); r.a += t.a; r.b += t.b; }
    va_end(ap);
    return r;
}
static struct ll sum_ll(int n, ...) {
    va_list ap; va_start(ap, n);
    struct ll r = {0, 0};
    for (int i = 0; i < n; i++) { struct ll t = va_arg(ap, struct ll); r.a += t.a; r.b += t.b; }
    va_end(ap);
    return r;
}
static struct big sum_big(int n, ...) {
    va_list ap; va_start(ap, n);
    struct big r = {0, 0, 0, 0};
    for (int i = 0; i < n; i++) {
        struct big t = va_arg(ap, struct big);
        r.a += t.a; r.b += t.b; r.c += t.c; r.d += t.d;
    }
    va_end(ap);
    return r;
}
static struct a16 sum_a16(int n, ...) {
    va_list ap; va_start(ap, n);
    struct a16 r = {0};
    for (int i = 0; i < n; i++) { struct a16 t = va_arg(ap, struct a16); r.x += t.x; }
    va_end(ap);
    return r;
}

/* ---- named struct params that consume SSE argument registers: fp_offset
 * must start AFTER them (regression for the fp_reg_count fix). ---- */
struct d1 { double x; };

static double after_dd(struct dd a, int tag, ...) {
    va_list ap; va_start(ap, tag);
    double s = 0;
    for (int i = 0; i < tag; i++) s += va_arg(ap, double);
    va_end(ap);
    return s + a.a + a.b;
}
static double after_dl(struct dl a, int tag, ...) {
    va_list ap; va_start(ap, tag);
    double s = 0;
    for (int i = 0; i < tag; i++) s += va_arg(ap, double);
    va_end(ap);
    return s + a.d + (double)a.l;
}
static double after_d1(struct d1 a, int tag, ...) {
    va_list ap; va_start(ap, tag);
    double s = 0;
    for (int i = 0; i < tag; i++) s += va_arg(ap, double);
    va_end(ap);
    return s + a.x;
}

/* ---- va_copy independence ---- */
static int copy_check(int n, ...) {
    va_list ap, ap2;
    va_start(ap, n);
    va_copy(ap2, ap);
    int a = 0, b = 0;
    for (int i = 0; i < n; i++) a += va_arg(ap, int);
    for (int i = 0; i < n; i++) b += va_arg(ap2, int);
    va_end(ap2);
    va_end(ap);
    return a != b || a != (n * (n + 1)) / 2;
}

int main(void) {
    int rc = 0;

    if (check_ld(0, 0x1p+63L + 1.0L)) { printf("FAIL ld\n"); rc |= 1; }

    if (isum(2, ((__int128)1 << 100) + 5, (__int128)7) != (((__int128)1 << 100) + 12)) {
        printf("FAIL i128\n"); rc |= 2;
    }
    if ((__int128)usum(2, ((unsigned __int128)1 << 127) + 3, (unsigned __int128)9) !=
        (__int128)(((unsigned __int128)1 << 127) + 12)) {
        printf("FAIL u128\n"); rc |= 4;
    }

    {
        struct dl r = sum_dl(3, (struct dl){1.5, 100}, (struct dl){2.5, 200}, (struct dl){3.5, 300});
        if (r.d != 7.5 || r.l != 600) { printf("FAIL dl\n"); rc |= 8; }
    }
    {
        struct ld r = sum_lds(3, (struct ld){100, 1.5}, (struct ld){200, 2.5}, (struct ld){300, 3.5});
        if (r.l != 600 || r.d != 7.5) { printf("FAIL ld2\n"); rc |= 16; }
    }
    {
        struct dd r = sum_dd(4, (struct dd){1,2}, (struct dd){3,4}, (struct dd){5,6}, (struct dd){7,8});
        if (r.a != 16.0 || r.b != 20.0) { printf("FAIL dd\n"); rc |= 32; }
    }
    {
        struct ll r = sum_ll(3, (struct ll){1,2}, (struct ll){3,4}, (struct ll){5,6});
        if (r.a != 9 || r.b != 12) { printf("FAIL ll\n"); rc |= 64; }
    }
    {
        struct big r = sum_big(3, (struct big){1,2,3,4}, (struct big){5,6,7,8}, (struct big){9,10,11,12});
        if (r.a != 15 || r.b != 18 || r.c != 21 || r.d != 24) { printf("FAIL big\n"); rc |= 128; }
    }
    {
        struct a16 r = sum_a16(8, (struct a16){1}, (struct a16){2}, (struct a16){3}, (struct a16){4},
                                  (struct a16){5}, (struct a16){6}, (struct a16){7}, (struct a16){8});
        if (r.x != 36) { printf("FAIL a16\n"); rc |= 256; }
    }

    if (copy_check(4, 1, 2, 3, 4)) { printf("FAIL copy\n"); rc |= 512; }

    /* named SSE-class struct params shift fp_offset by their SSE registers */
    if (after_dd((struct dd){10, 20}, 3, 1.5, 2.5, 3.5) != 37.5)  { printf("FAIL after_dd\n"); rc |= 1024; }
    if (after_dl((struct dl){1.25, 100}, 3, 1.5, 2.5, 3.5) != 108.75) { printf("FAIL after_dl\n"); rc |= 2048; }
    if (after_d1((struct d1){2.75}, 3, 1.5, 2.5, 3.5) != 10.25)  { printf("FAIL after_d1\n"); rc |= 4096; }

    printf(rc ? "FAIL va_arg_wide_struct\n" : "OK va_arg_wide_struct\n");
    return rc != 0;
}
