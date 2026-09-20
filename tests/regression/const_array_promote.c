/*
 * Constant-array promotion: runtime semantics battery.
 *
 * const_array_promote turns local arrays whose surviving stores are all
 * constants tiling the object exactly — and whose address only feeds
 * loads and provably read-only direct callees — into .rodata globals.
 * This battery drives every accepted and rejected shape with concrete
 * values: each positive shape's expected values are computed from
 * first principles in the test (no reference to the array's own
 * initialization), and each adversarial shape must keep exact C
 * semantics whether or not the promotion fired (the structural
 * promote/no-promote contracts live in check_const_array_promote.sh).
 *
 * Exit code 0 = every check agreed. The printed hash allows the suite's
 * GCC differential to catch oracle-side mistakes.
 */
#include <stdio.h>
#include <string.h>

#define NOINLINE __attribute__((noinline))

/* ------------------------------------------------------------------ *
 * Callee classification helpers.
 * ------------------------------------------------------------------ */

NOINLINE static int sum_u8(const unsigned char *p, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) s += p[i];
    return s;
}
NOINLINE static int sum_u16(const unsigned short *p, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) s += p[i];
    return s;
}
NOINLINE static int sum_i32(const int *p, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) s += p[i];
    return s;
}
NOINLINE static long long sum_i64(const long long *p, int n) {
    long long s = 0;
    for (int i = 0; i < n; i++) s += p[i];
    return s;
}
/* Writes through its parameter: any array passed here must NOT promote. */
NOINLINE static int write_through(int *p, int n) {
    for (int i = 0; i < n; i++) p[i] = p[i] + 1;
    return p[0];
}
/* Phi-free two-element reader: provably read-only with no loop at all. */
NOINLINE static int rd_two(const int *p) { return p[0] + p[1]; }
/* Extern (unknown body): arrays passed here must NOT promote. */
int opaque_reader(const int *p, int n);

/* ------------------------------------------------------------------ *
 * Positive shapes: promote and stay exact.
 * ------------------------------------------------------------------ */

/* P1: the vecreg shape — byte array, affine init loop, read-only callee. */
static int p1(void) {
    unsigned char a[16];
    for (int i = 0; i < 16; i++) a[i] = (unsigned char)(i * 17 + 3);
    return sum_u8(a, 16);
}
static int p1_ref(void) {
    int s = 0;
    for (int i = 0; i < 16; i++) s += (i * 17 + 3) & 0xff;
    return s;
}

/* P2: dword array, direct loads only (no call). */
static int p2(void) {
    int a[8];
    for (int i = 0; i < 8; i++) a[i] = i * i - 3 * i + 7;
    return a[0] + a[3] * 2 + a[7] * 4;
}
static int p2_ref(void) {
    int s = 0;
    for (int i = 0; i < 8; i++) {
        int v = i * i - 3 * i + 7;
        if (i == 0) s += v;
        if (i == 3) s += 2 * v;
        if (i == 7) s += 4 * v;
    }
    return s;
}

/* P3: qword array with a negative constant and odd access order. */
static long long p3(void) {
    long long a[4] = {0, 0, 0, 0};
    a[0] = -1000000000000LL;
    a[1] = 1;
    a[2] = 999999999999LL;
    a[3] = -1;
    return a[3] + a[1] * 2 + a[2] + a[0];
}
static long long p3_ref(void) { return -1 + 2 + 999999999999LL - 1000000000000LL; }

/* P4: mixed-width stores tiling exactly (1+1+2+4+4+4 bytes = 16). */
static int p4(void) {
    unsigned char buf[16];
    buf[0] = 0x11;
    buf[1] = 0x22;
    *(unsigned short *)(buf + 2) = 0x3344;
    *(unsigned int *)(buf + 4) = 0x55667788u;
    *(unsigned int *)(buf + 8) = 0x99aabbccu;
    *(unsigned int *)(buf + 12) = 0xddeeff00u;
    return sum_u8(buf, 16);
}
static int p4_ref(void) {
    return 0x11 + 0x22 + 0x44 + 0x33 + 0x88 + 0x77 + 0x66 + 0x55 + 0xcc +
           0xbb + 0xaa + 0x99 + 0x00 + 0xff + 0xee + 0xdd;
}

/* P5: word array, read-only callee. */
static int p5(void) {
    unsigned short w[6];
    for (int i = 0; i < 6; i++) w[i] = (unsigned short)(60000u + i * 7u);
    return sum_u16(w, 6);
}
static int p5_ref(void) {
    int s = 0;
    for (int i = 0; i < 6; i++) s += (60000u + (unsigned)i * 7u) & 0xffff;
    return s;
}

/* P6: two read-only callees, one array. */
static int p6(void) {
    int a[4];
    for (int i = 0; i < 4; i++) a[i] = 10 - i;
    return sum_i32(a, 4) - sum_i32(a, 4) / 2;
}
static int p6_ref(void) { return (10 + 9 + 8 + 7) - (10 + 9 + 8 + 7) / 2; }

/* P7: interior pointer argument (&a[3]) to a read-only callee. */
static int p7(void) {
    int a[6];
    for (int i = 0; i < 6; i++) a[i] = i * 100;
    return sum_i32(a + 3, 3);
}
static int p7_ref(void) { return 300 + 400 + 500; }

/* P8: the pointer-induction unlock. sum_i32's reader loop is
 * strength-reduced at -O2 (the scalar remainder walks a pointer phi
 * merging the preheader pointer and the backedge GEP), so promoting this
 * shape is legal ONLY because the callee proof tracks phis of derived
 * pointers AND the caller audit classifies the address by provenance.
 * Before both, this shape (and every noinline i32 reader helper) stayed
 * an alloca with 4 scalar stores. */
static int p8(void) {
    int a[4];
    for (int i = 0; i < 4; i++) a[i] = i * 3;
    return sum_i32(a, 4);
}
static int p8_ref(void) { return 0 + 3 + 6 + 9; }

/* P9: alignment is preserved — an int[2] (8 bytes, natural align 4) must
 * land in .rodata at .align 4, not 1 (the gate pins the directive). */
static int p9(void) {
    int a[2];
    a[0] = 11;
    a[1] = 22;
    return a[0] * 10 + a[1];
}
static int p9_ref(void) { return 110 + 22; }

/* P10: explicit alignment is honored (_Alignas(64) -> .align 64). */
static int p10(void) {
    _Alignas(64) int a[2];
    a[0] = 5;
    a[1] = 6;
    return a[0] + a[1];
}
static int p10_ref(void) { return 11; }

/* ------------------------------------------------------------------ *
 * Adversarial shapes: the promotion must reject (or stay sound);
 * results must be exact C semantics regardless.
 * ------------------------------------------------------------------ */

/* A1: runtime store after constant init — must NOT promote. */
static int a1(int k, int v) {
    int a[4];
    for (int i = 0; i < 4; i++) a[i] = i;
    a[k & 3] = v;
    return sum_i32(a, 4);
}
static int a1_ref(int k, int v) {
    int s = 0;
    for (int i = 0; i < 4; i++) s += (i == (k & 3)) ? v : i;
    return s;
}

/* A1b: the same runtime store with a callee that is provably read-only
 * WITHOUT any pointer phi — the store rejection must not depend on the
 * callee's loop shape. (This is the hole that shipped: the variable-offset
 * store was invisible to the const-offset-only classifier, and with a
 * phi-free callee the array promoted and the store landed in .rodata.) */
static int a1b(int k, int v) {
    int a[4];
    for (int i = 0; i < 4; i++) a[i] = i;
    a[k & 3] = v;
    return rd_two(a);
}
static int a1b_ref(int k, int v) {
    int i = k & 3;
    return (i == 0 ? v : 0) + (i == 1 ? v : 1);
}

/* A1c: the store through a pointer COPY (and a cast spelling) — the same
 * provenance rule, different dataflow spelling. */
static int a1c(int k, int v) {
    int a[4];
    for (int i = 0; i < 4; i++) a[i] = i;
    int *p = (int *)((char *)a);
    p[k & 3] = v;
    return sum_i32(a, 4);
}
static int a1c_ref(int k, int v) {
    int s = 0;
    for (int i = 0; i < 4; i++) s += (i == (k & 3)) ? v : i;
    return s;
}

/* A2: partial initialization (gap) — must NOT promote (uninit bytes). */
static int a2(void) {
    unsigned char a[8];
    a[0] = 1;
    a[2] = 3;
    a[4] = 5;
    a[6] = 7;
    /* a[1], a[3], a[5], a[7] intentionally never written; read only the
     * written ones so the result is defined either way. */
    return a[0] + a[2] + a[4] + a[6];
}
static int a2_ref(void) { return 1 + 3 + 5 + 7; }

/* A3: address escapes into memory — must NOT promote. */
static int a3(void) {
    static const int *keep;
    int a[3];
    for (int i = 0; i < 3; i++) a[i] = i + 5;
    keep = a;
    return keep[0] + keep[1] + keep[2];
}
static int a3_ref(void) { return 5 + 6 + 7; }

/* A4: callee writes through the parameter — must NOT promote. */
static int a4(void) {
    int a[4];
    for (int i = 0; i < 4; i++) a[i] = i;
    write_through(a, 4);
    return sum_i32(a, 4);
}
static int a4_ref(void) {
    int s = 0;
    for (int i = 0; i < 4; i++) s += i + 1;
    return s;
}

/* A5: conditional initialization — must NOT promote. */
static int a5(int c) {
    int a[4];
    int s = 0;
    if (c) {
        for (int i = 0; i < 4; i++) a[i] = i * 2;
        s += sum_i32(a, 4);
    }
    for (int i = 0; i < 4; i++) a[i] = i * 3;
    s += sum_i32(a, 4);
    return s;
}
static int a5_ref(int c) {
    int s = 0;
    if (c) s += 0 + 2 + 4 + 6;
    s += 0 + 3 + 6 + 9;
    return s;
}

/* A6: overlapping stores — the later value must win, so no promotion. */
static int a6(void) {
    unsigned int a[2];
    *(unsigned short *)&a[0] = 0x1122u;
    a[0] = 0x33445566u; /* overwrites the half-word store */
    *(unsigned short *)&a[1] = 0x7788u;
    a[1] = a[1] & 0xffffu;
    return (int)(a[0] & 0xff) + (int)((a[0] >> 24) & 0xff) + (int)(a[1] & 0xff);
}
static int a6_ref(void) { return 0x66 + 0x33 + 0x88; }

/* A7: init stores separated across blocks (if/else both storing) —
 * sound either way; results must be exact. */
static int a7(int c) {
    unsigned char a[4];
    if (c) {
        a[0] = 1; a[1] = 2; a[2] = 3; a[3] = 4;
    } else {
        a[0] = 5; a[1] = 6; a[2] = 7; a[3] = 8;
    }
    return sum_u8(a, 4);
}
static int a7_ref(int c) { return c ? 10 : 26; }

/* A8: the address escapes through a TERMINATOR (return). Dangling by the
 * C rules, but the compiler must lower it without an ICE, and the
 * promotion must reject: the rewrite would delete the alloca out from
 * under the returned reference. (Found live: this ICE'd the backend
 * before terminators were audited.) */
NOINLINE static int *leaky(void) {
    int a[4];
    for (int i = 0; i < 4; i++) a[i] = i;
    return a;
}
static int a8(void) {
    int *p = leaky();
    return (p != 0) ? 13 : 4;
}
static int a8_ref(void) { return 13; }

/* ------------------------------------------------------------------ */

static int fails = 0;

#define CHECK(expr, ref, ctx)                                                  \
    do {                                                                       \
        int _e = (expr);                                                       \
        int _r = (ref);                                                        \
        if (_e != _r) {                                                        \
            printf("MISMATCH %s ctx=%d: got %d want %d\n", #expr, (int)(ctx),  \
                   _e, _r);                                                    \
            fails++;                                                           \
        }                                                                      \
    } while (0)

int main(void) {
    unsigned long long h = 1469598103934665603ULL;

    CHECK(p1(), p1_ref(), 1);
    CHECK(p2(), p2_ref(), 2);
    CHECK(p3() == p3_ref(), 1, 3);
    CHECK(p4(), p4_ref(), 4);
    CHECK(p5(), p5_ref(), 5);
    CHECK(p6(), p6_ref(), 6);
    CHECK(p7(), p7_ref(), 7);
    CHECK(p8(), p8_ref(), 81);
    CHECK(p9(), p9_ref(), 82);
    CHECK(p10(), p10_ref(), 83);
    CHECK(a1(0, 42), a1_ref(0, 42), 8);
    CHECK(a1(2, -7), a1_ref(2, -7), 9);
    CHECK(a1b(0, 42), a1b_ref(0, 42), 84);
    CHECK(a1b(1, -7), a1b_ref(1, -7), 85);
    CHECK(a1b(3, 5), a1b_ref(3, 5), 86);
    CHECK(a1c(0, 42), a1c_ref(0, 42), 87);
    CHECK(a1c(2, -7), a1c_ref(2, -7), 88);
    CHECK(a2(), a2_ref(), 10);
    CHECK(a3(), a3_ref(), 11);
    CHECK(a4(), a4_ref(), 12);
    CHECK(a5(0), a5_ref(0), 13);
    CHECK(a5(1), a5_ref(1), 14);
    CHECK(a6(), a6_ref(), 15);
    CHECK(a7(0), a7_ref(0), 16);
    CHECK(a7(1), a7_ref(1), 17);
    CHECK(a8(), a8_ref(), 89);
    /* Every start value of a1/a1b/a1c: the runtime store hits each index. */
    for (int k = 0; k < 4; k++) {
        CHECK(a1(k, 100 + k), a1_ref(k, 100 + k), 20 + k);
        CHECK(a1b(k, 100 + k), a1b_ref(k, 100 + k), 30 + k);
        CHECK(a1c(k, 100 + k), a1c_ref(k, 100 + k), 40 + k);
    }

    h = h * 31 + (unsigned)p1() + (unsigned)p2() + (unsigned)p4() +
        (unsigned)p5() + (unsigned)p8() + (unsigned)p9() + (unsigned)p10() +
        (unsigned)a1(1, 5) + (unsigned)a1b(2, 9) + (unsigned)a1c(3, 4) +
        (unsigned)a8();

    if (fails) {
        printf("const_array_promote: %d mismatches\n", fails);
        return 1;
    }
    printf("const_array_promote ok h=%llu\n", h);
    return 0;
}
