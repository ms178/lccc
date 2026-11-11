/* Parameters of a function whose body does 128-bit arithmetic.

   On x86-64 the 128-bit emitters stage through %rdx everywhere and through
   %rdi/%rsi/%r8/%r9 at their own instructions, so such a body cannot park its
   parameters in the argument registers the way a narrow body does.  They
   used to take callee-saved registers instead -- `mul_uu` below pushed %rbx
   and %r12 for two values it reads once.  They now fill the free stretches of
   %r10/%r11 (the allocator's Phase 2-x64wp), which no argument arrives in and
   no 128-bit emitter writes.

   Every function here is a shape that wave has to get right: a parameter that
   dies before the wide instruction, one that lives across it, one that is the
   base of constant-offset accesses (%r11 cannot be), one that is an argument
   of an indirect call (%r10 holds the target), one that spans a call, more
   parameters than the pair can hold, and stack-passed ones.

   The expected values are computed with 64-bit arithmetic only. */
#include <stdio.h>

#ifdef __SIZEOF_INT128__

#define NOINLINE __attribute__((noinline, noclone))
typedef unsigned long long u64;
typedef unsigned __int128 u128;

static int failures;

static void expect(const char *what, u64 got, u64 want) {
    if (got != want) {
        printf("%s: got %llx, want %llx\n", what, got, want);
        failures++;
    }
}

/* High half of a 64x64 product from 32-bit limbs: no 128-bit value. */
NOINLINE static u64 ref_mulhi(u64 a, u64 b) {
    u64 al = a & 0xffffffffu, ah = a >> 32, bl = b & 0xffffffffu, bh = b >> 32;
    u64 ll = al * bl, lh = al * bh, hl = ah * bl, hh = ah * bh;
    u64 mid = (ll >> 32) + (lh & 0xffffffffu) + (hl & 0xffffffffu);
    return hh + (lh >> 32) + (hl >> 32) + (mid >> 32);
}

static u64 lo(u128 v) { return (u64)v; }
static u64 hi(u128 v) { return (u64)(v >> 64); }

/* Both parameters die at their widening casts, before the multiply. */
NOINLINE u128 mul_uu(u64 a, u64 b) { return (u128)a * b; }
NOINLINE u128 sq_u(u64 a) { return (u128)a * a; }

/* Two pointers live across a wide load and a wide store. */
NOINLINE void copyu(u128 *d, const u128 *s) { *d = *s; }
NOINLINE u128 addu(const u128 *p, const u128 *q) { return *p + *q; }
NOINLINE void incu(u128 *p, u64 by) { *p += by; }

/* Four parameters: the third arrives in %rdx, the high half of every wide
   result, and the fourth in %rcx, the emitters' scratch. */
NOINLINE u128 mac(u64 a, u64 b, u64 c, u64 d) { return (u128)a * b + c + d; }

/* All six argument registers, each read after a wide instruction. */
NOINLINE u128 six(u64 a, u64 b, u64 c, u64 d, u64 e, u64 f) {
    u128 t = (u128)a * f;
    t += (u128)b * e;
    t += c;
    return t + d;
}

/* Eight: two arrive on the stack. */
NOINLINE u128 eight(u64 a, u64 b, u64 c, u64 d, u64 e, u64 f, u64 g, u64 h) {
    u128 t = (u128)a * h + g;
    t += (u128)b * c;
    return t + d + e + f;
}

struct acc {
    u64 a;
    u128 w;
    u64 b;
};

/* `s` is the base of constant-offset accesses on both sides of the wide
   instructions, and lives in a loop. */
NOINLINE u64 fields(struct acc *s, u64 k, int rounds) {
    for (int i = 0; i < rounds; i++) {
        s->w += (u128)s->a * k;
        s->b ^= k + (u64)i;
        s->a += 3;
    }
    return hi(s->w) ^ s->b;
}

static u64 mix(u64 x, u64 y) { return x * 31 + (y ^ 0x9e3779b97f4a7c15ull); }
static u64 (*volatile mixer)(u64, u64) = mix;

/* `a` and `b` are arguments of an indirect call and are read again after
   it; the wide product is computed before the call and consumed after. */
NOINLINE u64 with_call(u64 a, u64 b, u64 (*f)(u64, u64)) {
    u128 t = (u128)a * b;
    u64 r = f(a, b);
    return r + hi(t) + lo(t) + a - b;
}

/* The same without anything live across the call: `a` and `b` are free to
   take caller-saved homes, and one of them is an indirect-call argument. */
NOINLINE u64 call_last(u64 a, u64 b, u64 (*f)(u64, u64)) {
    u128 t = (u128)a * b;
    return f(hi(t) ^ a, lo(t) + b);
}

/* A carry-propagating multiply: every parameter lives across the loop. */
NOINLINE u64 mul_word(u64 *r, const u64 *a, int n, u64 w) {
    u64 carry = 0;
    for (int i = 0; i < n; i++) {
        u128 t = (u128)a[i] * w + r[i] + carry;
        r[i] = lo(t);
        carry = hi(t);
    }
    return carry;
}

/* A variable shift: the count goes through %rcx. */
NOINLINE void shifted(u128 *p, unsigned n, u64 x) { *p = (*p << (n & 63)) | x; }

/* A wide compare whose result selects between two parameters. */
NOINLINE u64 pick(const u128 *p, const u128 *q, u64 a, u64 b) { return *p < *q ? a : b + 1; }

/* A library call (128-bit division) between the parameters' uses. */
NOINLINE u64 divmix(u64 a, u64 b, u64 c) {
    u128 t = (u128)a * b / (c | 1);
    return lo(t) + a + (b ^ c);
}

/* A narrow division next to the wide arithmetic: %rdx twice over. */
NOINLINE u64 narrow_div(u64 a, u64 b, u64 c) {
    u128 t = (u128)a * b;
    return hi(t) % (c | 1) + lo(t) / (c | 1) + c;
}

int main(void) {
    static const u64 seeds[] = {
        0, 1, 2, 0xffffffffull, 0x100000000ull, 0x123456789abcdef0ull, 0x8000000000000000ull,
        0xfedcba9876543210ull, 0xffffffffffffffffull,
    };
    enum { N = sizeof seeds / sizeof seeds[0] };

    for (int i = 0; i < N; i++) {
        for (int j = 0; j < N; j++) {
            u64 a = seeds[i], b = seeds[j];
            u64 plo = a * b, phi = ref_mulhi(a, b);
            u128 m = mul_uu(a, b);
            expect("mul_uu.lo", lo(m), plo);
            expect("mul_uu.hi", hi(m), phi);

            u64 c = seeds[(i + j) % N] ^ 0x55, d = seeds[(i * 3 + j) % N] + 7;
            u64 s1 = plo + c, c1 = s1 < plo;
            u64 s2 = s1 + d, c2 = s2 < s1;
            m = mac(a, b, c, d);
            expect("mac.lo", lo(m), s2);
            expect("mac.hi", hi(m), phi + c1 + c2);

            u64 e = seeds[(j + 4) % N], f = seeds[(i + 5) % N] | 1;
            u64 q1lo = a * f, q1hi = ref_mulhi(a, f), q2lo = b * e, q2hi = ref_mulhi(b, e);
            u64 t0 = q1lo + q2lo, k0 = t0 < q1lo;
            u64 t1 = t0 + c, k1 = t1 < t0;
            u64 t2 = t1 + d, k2 = t2 < t1;
            m = six(a, b, c, d, e, f);
            expect("six.lo", lo(m), t2);
            expect("six.hi", hi(m), q1hi + q2hi + k0 + k1 + k2);

            u64 g = seeds[(i + 2) % N], h = seeds[(j + 7) % N];
            u64 x1lo = a * h, x1hi = ref_mulhi(a, h), x2lo = b * c, x2hi = ref_mulhi(b, c);
            u64 w = x1lo + g, wc = w < x1lo;
            u64 w2 = w + x2lo, wc2 = w2 < w;
            u64 w3 = w2 + d, wc3 = w3 < w2;
            u64 w4 = w3 + e, wc4 = w4 < w3;
            u64 w5 = w4 + f, wc5 = w5 < w4;
            m = eight(a, b, c, d, e, f, g, h);
            expect("eight.lo", lo(m), w5);
            expect("eight.hi", hi(m), x1hi + x2hi + wc + wc2 + wc3 + wc4 + wc5);

            expect("with_call", with_call(a, b, mixer), mix(a, b) + phi + plo + a - b);
            expect("call_last", call_last(a, b, mixer), mix(phi ^ a, plo + b));

            u64 dv = c | 1;
            expect("narrow_div", narrow_div(a, b, c), phi % dv + plo / dv + c);
            if (phi == 0)
                expect("divmix", divmix(a, b, c), plo / dv + a + (b ^ c));
        }
        u128 sq = sq_u(seeds[i]);
        expect("sq_u.lo", lo(sq), seeds[i] * seeds[i]);
        expect("sq_u.hi", hi(sq), ref_mulhi(seeds[i], seeds[i]));
    }

    u128 x = ((u128)0x1111222233334444ull << 64) | 0xfffffffffffffff0ull;
    u128 y = ((u128)0x9999aaaabbbbccccull << 64) | 0x0000000000000021ull;
    u128 z = 0;
    copyu(&z, &x);
    expect("copyu.lo", lo(z), 0xfffffffffffffff0ull);
    expect("copyu.hi", hi(z), 0x1111222233334444ull);
    z = addu(&x, &y);
    expect("addu.lo", lo(z), 0x11);
    expect("addu.hi", hi(z), 0x1111222233334444ull + 0x9999aaaabbbbccccull + 1);
    incu(&x, 0x20);
    expect("incu.lo", lo(x), 0x10);
    expect("incu.hi", hi(x), 0x1111222233334445ull);
    shifted(&y, 64 + 8, 0xa5);
    expect("shifted.lo", lo(y), 0x21a5);
    expect("shifted.hi", hi(y), 0x99aaaabbbbcccc00ull);
    expect("pick.lt", pick(&x, &y, 3, 9), 3);
    expect("pick.ge", pick(&y, &x, 3, 9), 10);
    expect("pick.eq", pick(&x, &x, 3, 9), 10);

    struct acc s = {0xffffffffffffff00ull, 5, 0x77};
    u64 wl = 5, wh = 0, sb = 0x77, sa = 0xffffffffffffff00ull;
    for (int i = 0; i < 9; i++) {
        u64 pl = sa * 0x123456789ull, ph = ref_mulhi(sa, 0x123456789ull);
        u64 nl = wl + pl;
        wh += ph + (nl < wl);
        wl = nl;
        sb ^= 0x123456789ull + (u64)i;
        sa += 3;
    }
    expect("fields", fields(&s, 0x123456789ull, 9), wh ^ sb);
    expect("fields.w.lo", lo(s.w), wl);
    expect("fields.a", s.a, sa);

    u64 r[5] = {1, 0xffffffffffffffffull, 3, 0x8000000000000000ull, 5};
    const u64 av[5] = {0xffffffffffffffffull, 2, 0x123456789abcdef0ull, 7, 0xfedcba9876543210ull};
    u64 want[5], carry = 0;
    for (int i = 0; i < 5; i++) {
        u64 pl = av[i] * 0xdeadbeefcafef00dull, ph = ref_mulhi(av[i], 0xdeadbeefcafef00dull);
        u64 t = pl + r[i];
        ph += t < pl;
        u64 t2 = t + carry;
        ph += t2 < t;
        want[i] = t2;
        carry = ph;
    }
    expect("mul_word", mul_word(r, av, 5, 0xdeadbeefcafef00dull), carry);
    for (int i = 0; i < 5; i++)
        expect("mul_word.r", r[i], want[i]);

    printf("%s\n", failures ? "FAIL" : "ok");
    return failures != 0;
}

#else

int main(void) {
    printf("ok\n");
    return 0;
}

#endif
