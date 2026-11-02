/* Two-register aggregate returns (9..=16 bytes, RAX:RDX / x0:x1 / a0:a1)
 * and the I128 carrier they travel in.
 *
 * Covers:
 *  - every carrier size 9..16, inlined (pack/unpack meet and must fold away)
 *    and out of line (the real ABI path), with guard words around the
 *    destination so an over-wide high-half store is caught (the call-site
 *    unpack used to store a full 8-byte high half into 9..15-byte objects);
 *  - odd remainders (11, 13, 14, 15 bytes) that take the rounded carrier;
 *  - signed members, so a sign-extended low half is never mistaken for a
 *    zero-extended one;
 *  - __int128 bodies under register pressure (wide multiply, a wide compare
 *    fused into its branch while narrow values stay live across it, wide
 *    Copy/phi flows, a struct-returning helper with a hot loop), which
 *    exercise the position-aware r8/r9/rdi/rsi admission for wide bodies.
 *
 * Self-checking: exit 0 = pass. Output is deterministic for the GCC A/B.
 */
#include <stdio.h>
#include <string.h>

#define NOI __attribute__((noinline))
#define AI static inline __attribute__((always_inline))

static int fails;
#define CHECK(c)                                                              \
    do {                                                                      \
        if (!(c)) {                                                           \
            printf("FAIL line %d: %s\n", __LINE__, #c);                       \
            fails++;                                                          \
        }                                                                     \
    } while (0)

#define GUARD 0xA5A5A5A5u

#define DEF_SIZE(N)                                                           \
    typedef struct {                                                          \
        unsigned char b[N];                                                   \
    } S##N;                                                                   \
    AI S##N mk_i##N(unsigned seed)                                            \
    {                                                                         \
        S##N s;                                                               \
        for (int i = 0; i < N; i++)                                           \
            s.b[i] = (unsigned char)(seed * 31u + (unsigned)i * 7u + 1u);     \
        return s;                                                             \
    }                                                                         \
    NOI S##N mk_o##N(unsigned seed) { return mk_i##N(seed); }                 \
    NOI int check##N(unsigned seed)                                           \
    {                                                                         \
        struct {                                                              \
            unsigned lo;                                                      \
            S##N s;                                                           \
            unsigned hi;                                                      \
        } box;                                                                \
        int bad = 0;                                                          \
        box.lo = GUARD;                                                       \
        box.hi = GUARD;                                                       \
        box.s = mk_o##N(seed);                                                \
        for (int i = 0; i < N; i++)                                           \
            bad |= box.s.b[i] !=                                              \
                   (unsigned char)(seed * 31u + (unsigned)i * 7u + 1u);       \
        bad |= box.lo != GUARD || box.hi != GUARD;                            \
        S##N t = mk_i##N(seed + 1);                                           \
        S##N u = mk_o##N(seed + 1);                                           \
        bad |= memcmp(&t, &u, N) != 0;                                        \
        return bad;                                                           \
    }

DEF_SIZE(9)
DEF_SIZE(10)
DEF_SIZE(11)
DEF_SIZE(12)
DEF_SIZE(13)
DEF_SIZE(14)
DEF_SIZE(15)
DEF_SIZE(16)

/* zstd's repcodes_t shape: {U32 rep[3]}, built on two paths and merged. */
typedef struct {
    unsigned rep[3];
} Rep;

AI Rep new_rep(const unsigned *rep, unsigned off, unsigned ll0)
{
    Rep r;
    if (off > 3) {
        r.rep[2] = rep[1];
        r.rep[1] = rep[0];
        r.rep[0] = off - 3;
    } else {
        unsigned idx = off - 1 + ll0;
        if (idx != 0) {
            unsigned cur = idx == 3 ? rep[0] - 1 : rep[idx];
            r.rep[2] = idx >= 2 ? rep[1] : rep[2];
            r.rep[1] = rep[0];
            r.rep[0] = cur;
        } else {
            memcpy(&r, rep, sizeof r);
        }
    }
    return r;
}

NOI unsigned rep_sum(const unsigned *rep, unsigned off, unsigned ll0,
                     unsigned x, unsigned y, unsigned z, unsigned w)
{
    Rep r = new_rep(rep, off, ll0);
    return r.rep[0] * x + r.rep[1] * y + r.rep[2] * z + w;
}

/* Signed members: the low eightbyte holds negative values. */
typedef struct {
    int a, b;
    short c;
    signed char d;
} Sgn; /* 12 bytes with padding: 4 + 4 + 2 + 1 (+1) */

AI Sgn mk_sgn(int k)
{
    Sgn s = {-k, k - 100, (short)(-k * 3), (signed char)(-k)};
    return s;
}
NOI Sgn mk_sgn_o(int k) { return mk_sgn(k); }

NOI long sgn_mix(int k)
{
    Sgn a = mk_sgn(k), b = mk_sgn_o(k);
    return (long)a.a + a.b + a.c + a.d - ((long)b.a + b.b + b.c + b.d);
}

/* Wide bodies under pressure (64-bit targets: i686 has no __int128). */
#ifdef __SIZEOF_INT128__
typedef unsigned __int128 u128;

NOI unsigned long long sink(unsigned long long v) { return v ^ 0x5555; }

NOI unsigned long long wide_mix(const unsigned long long *p, int n,
                                unsigned long long k)
{
    unsigned long long acc = 0, a = 1, b = 2, c = 3, d = 4;
    for (int i = 0; i < n; i++) {
        unsigned long long v = p[i];
        a = a * 3 + v;
        b ^= v >> 3;
        c += a ^ b;
        d = d * 5 + (c >> 7);
        u128 m = (u128)v * k; /* wide multiply: r8/r9/rdi/rsi scratch */
        unsigned long long hi = (unsigned long long)(m >> 64);
        unsigned long long lo = (unsigned long long)m;
        if (m > ((u128)1 << 70)) /* wide compare fused into the branch */
            acc += hi + (a & 0xff);
        else
            acc += lo ^ d;
    }
    return acc + a + b + c + d + sink(acc);
}

typedef struct {
    unsigned long long lo, hi;
} Pair;

/* A struct-returning helper (I128 pack at its return) whose body runs a
 * hot narrow loop: the narrow loop values must stay correct whatever
 * registers they take. */
NOI Pair minmax(const unsigned long long *p, int n)
{
    unsigned long long mn = ~0ull, mx = 0, s = 0, t = 1;
    for (int i = 0; i < n; i++) {
        unsigned long long v = p[i];
        mn = v < mn ? v : mn;
        mx = v > mx ? v : mx;
        s += v;
        t = t * 33 + (v & 7);
    }
    Pair r = {mn ^ (s & 0), mx + (t & 0) };
    return r;
}

NOI u128 wide_phi(int sel, unsigned long long a, unsigned long long b)
{
    u128 x;
    if (sel)
        x = ((u128)a << 64) | b;
    else
        x = ((u128)b << 64) | a;
    return x + 1;
}
#endif /* __SIZEOF_INT128__ */

int main(void)
{
    for (unsigned s = 0; s < 4; s++) {
        CHECK(!check9(s));
        CHECK(!check10(s));
        CHECK(!check11(s));
        CHECK(!check12(s));
        CHECK(!check13(s));
        CHECK(!check14(s));
        CHECK(!check15(s));
        CHECK(!check16(s));
    }

    unsigned rp[3] = {10, 20, 30};
    CHECK(rep_sum(rp, 9, 0, 1, 1, 1, 0) == 6 + 10 + 20);
    CHECK(rep_sum(rp, 1, 0, 1, 1, 1, 0) == 60);
    CHECK(rep_sum(rp, 1, 1, 1, 2, 3, 4) == 20 + 2 * 10 + 3 * 30 + 4);
    CHECK(rep_sum(rp, 3, 1, 1, 1, 1, 0) == 9 + 10 + 20);

    CHECK(sgn_mix(7) == 0);
    Sgn sg = mk_sgn_o(9);
    CHECK(sg.a == -9 && sg.b == -91 && sg.c == -27 && sg.d == -9);

#ifdef __SIZEOF_INT128__
    unsigned long long buf[64];
    for (int i = 0; i < 64; i++)
        buf[i] = 0x9E3779B97F4A7C15ull * (unsigned long long)(i + 1);
    unsigned long long w = wide_mix(buf, 64, 0xD6E8FEB86659FD93ull);
    printf("wide_mix=%llu\n", w);

    Pair pr = minmax(buf, 64);
    unsigned long long mn = ~0ull, mx = 0;
    for (int i = 0; i < 64; i++) {
        mn = buf[i] < mn ? buf[i] : mn;
        mx = buf[i] > mx ? buf[i] : mx;
    }
    CHECK(pr.lo == mn && pr.hi == mx);

    u128 x = wide_phi(1, 5, ~0ull);
    CHECK((unsigned long long)(x >> 64) == 6 && (unsigned long long)x == 0);
    u128 y = wide_phi(0, 5, 7);
    CHECK((unsigned long long)(y >> 64) == 7 && (unsigned long long)y == 6);

#endif
    printf("%s\n", fails ? "FAILED" : "OK");
    return fails != 0;
}
