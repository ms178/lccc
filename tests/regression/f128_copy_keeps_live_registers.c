/* `*out = *in` for a long double (or a __float128) is one copy of the
   object's bytes: the backend fuses the load and the store.  The pair is two
   ordinary instructions to liveness, so every register that holds a value
   across it must survive, and the copy must stop at the end of the object.

   Two defects were pinned here, both older than this file:

   x86-64  the copy staged its addresses in %rdi and %rsi, where the
           allocator had put live integers.  `live6` returned garbage at
           -O1, -O2, -O3 and -Os.
   i686    16 bytes were copied for a 12-byte long double, so the member
           behind it was overwritten (`tail_survives`).

   The same source runs as the _nosse (integer relay) and _i386 variants.  No
   floating-point value crosses a call boundary, so -mno-sse can build it. */
#include <stdio.h>

#define NOINLINE __attribute__((noinline, noclone))
typedef unsigned long word;

struct boxed { long double value; unsigned tail; };
struct pair { long double first, second; };

/* Six integers live across the copy. */
NOINLINE word live6(word **xpp, long double **qq, long double *out) {
    word *xp = *xpp;
    word a = xp[0], b = xp[1], c = xp[2], d = xp[3], e = xp[4], f = xp[5];
    *out = **qq;
    return a + 3 * b + 5 * c + 7 * d + 11 * e + 13 * f;
}

/* Ten: more than the caller-saved pool, so some pointers live in slots and
   the copy has to stage an address. */
NOINLINE word live10(word **xpp, long double **qq, long double **outp) {
    word *xp = *xpp;
    word a = xp[0], b = xp[1], c = xp[2], d = xp[3], e = xp[4];
    word f = xp[5], g = xp[6], h = xp[7], i = xp[8], j = xp[9];
    **outp = **qq;
    return a + 3 * b + 5 * c + 7 * d + 11 * e + 13 * f + 17 * g + 19 * h + 23 * i + 29 * j;
}

/* Two copies in a row, crossing: the second reads what the first wrote. */
NOINLINE word live_swap(word *xp, struct pair *p, long double *tmp) {
    word a = xp[0], b = xp[1], c = xp[2], d = xp[3], e = xp[4], f = xp[5];
    *tmp = p->first;
    p->first = p->second;
    p->second = *tmp;
    return a ^ (b << 1) ^ (c << 2) ^ (d << 3) ^ (e << 4) ^ (f << 5);
}

/* The source is a local: its address is a frame offset, not a register. */
NOINLINE word live_local(word *xp, long double *out, int scale) {
    volatile long double local = (long double)scale + 0.5L;
    word a = xp[0], b = xp[1], c = xp[2], d = xp[3], e = xp[4], f = xp[5];
    long double copy = local;
    *out = copy;
    return a * 2 + b * 3 + c * 5 + d * 7 + e * 11 + f * 13;
}

/* Both ends of the copy are frame objects: an address has to be staged. */
NOINLINE void escape(void *p) { __asm__ volatile("" : : "r"(p) : "memory"); }
NOINLINE word live_frame(word *xp, long double *out, const long double *in) {
    long double first, second;
    escape(&first);
    escape(&second);
    word a = xp[0], b = xp[1], c = xp[2], d = xp[3], e = xp[4], f = xp[5];
    first = *in;
    second = first;
    *out = second;
    return a * 2 + b * 3 + c * 5 + d * 7 + e * 11 + f * 13;
}

/* The copy sits in a loop and the live values are loop-carried. */
NOINLINE word live_loop(const word *xp, long double *dst, long double *const *src, int n) {
    word s0 = xp[0], s1 = xp[1], s2 = xp[2], s3 = xp[3];
    for (int i = 0; i < n; i++) {
        dst[i] = *src[i];
        s0 += s1 ^ (word)i;
        s1 += s2 + 1;
        s2 ^= s3 << 1;
        s3 += s0 >> 3;
    }
    return s0 ^ s1 ^ s2 ^ s3;
}

/* The copy must stop at the end of the object. */
NOINLINE void tail_survives(struct boxed *box, const long double *q) { box->value = *q; }

#ifdef __SIZEOF_FLOAT128__
NOINLINE word live6_q(word **xpp, __float128 **qq, __float128 *out) {
    word *xp = *xpp;
    word a = xp[0], b = xp[1], c = xp[2], d = xp[3], e = xp[4], f = xp[5];
    *out = **qq;
    return a + 3 * b + 5 * c + 7 * d + 11 * e + 13 * f;
}
#endif

static int failures;
static void expect(const char *what, word got, word want) {
    printf("%s %lu%s\n", what, got, got == want ? "" : " MISMATCH");
    failures += got != want;
}

int main(void) {
    word x[10] = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10};
    word *xp = x;
    long double in = 1000.0L, out = 0, *q = &in, *o = &out;

    expect("live6", live6(&xp, &q, &out), 183);
    expect("live6.out", out == 1000.0L, 1);

    out = 0;
    expect("live10", live10(&xp, &q, &o), 951);
    expect("live10.out", out == 1000.0L, 1);

    struct pair p = {2.5L, -7.25L};
    long double tmp = 0;
    expect("swap", live_swap(x, &p, &tmp), 1 ^ 4 ^ 12 ^ 32 ^ 80 ^ 192);
    expect("swap.values", p.first == -7.25L && p.second == 2.5L && tmp == 2.5L, 1);

    out = 0;
    expect("local", live_local(x, &out, 41), 2 + 6 + 15 + 28 + 55 + 78);
    expect("local.out", out == 41.5L, 1);

    out = 0;
    expect("frame", live_frame(x, &out, &in), 2 + 6 + 15 + 28 + 55 + 78);
    expect("frame.out", out == 1000.0L, 1);

    long double from[5] = {1.5L, 2.5L, 3.5L, 4.5L, 5.5L}, to[5] = {0};
    long double *const ptrs[5] = {&from[4], &from[3], &from[2], &from[1], &from[0]};
    word mixed = live_loop(x, to, ptrs, 5);
    word s0 = 1, s1 = 2, s2 = 3, s3 = 4;
    for (int i = 0; i < 5; i++) {
        s0 += s1 ^ (word)i;
        s1 += s2 + 1;
        s2 ^= s3 << 1;
        s3 += s0 >> 3;
    }
    expect("loop", mixed, s0 ^ s1 ^ s2 ^ s3);
    expect("loop.values", to[0] == 5.5L && to[1] == 4.5L && to[2] == 3.5L && to[3] == 2.5L && to[4] == 1.5L, 1);

    struct boxed box = {0.0L, 0x12345678u};
    struct boxed donor = {1.5L, 0xdeadbeefu};
    tail_survives(&box, &donor.value);
    expect("tail", box.tail, 0x12345678u);
    expect("tail.value", box.value == 1.5L, 1);

#ifdef __SIZEOF_FLOAT128__
    __float128 qin, qout, *qp = &qin;
    unsigned char *raw = (unsigned char *)&qin, *rawout = (unsigned char *)&qout;
    for (unsigned i = 0; i < sizeof qin; i++) {
        raw[i] = (unsigned char)(0xa5u + 17u * i);
        rawout[i] = 0;
    }
    expect("live6_q", live6_q(&xp, &qp, &qout), 183);
    expect("live6_q.out", __builtin_memcmp(&qin, &qout, sizeof qin) == 0, 1);
#endif

    printf("%s\n", failures ? "FAIL" : "ok");
    return failures != 0;
}
