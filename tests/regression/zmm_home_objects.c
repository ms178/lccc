/* 512-bit vector objects that live in registers.
 *
 * lccc used to keep every `__m512i` in 64 bytes of stack and copied it
 * through memory at each inlined call.  An object that is only read and
 * written whole now lives in one of zmm16..zmm31 (src/backend/
 * x86_zmm_homes.rs).  Each function below is one way that can go wrong: a
 * copy whose source changes afterwards, values that meet at a join, a
 * cycle of copies in a loop, more values than there are registers, a call
 * that overwrites every register, a result that replaces one of its own
 * sources, an object whose address is taken half way through.  The
 * expected values are computed lane by lane in C.
 */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

#define NOINLINE __attribute__((noinline))

static int failures;

static void expect(const char *what, const void *got, const void *want, size_t bytes)
{
    if (memcmp(got, want, bytes) != 0) {
        printf("FAIL %s\n", what);
        failures++;
    }
}

static __m512i load(const int *p) { return _mm512_loadu_si512((const void *)p); }
static void store(int *p, __m512i v) { _mm512_storeu_si512((void *)p, v); }

/* Whoever is called may write every vector register.  This one does. */
NOINLINE static void scribble(void)
{
    __asm__ volatile(
        "vpternlogd $0xff, %%zmm16, %%zmm16, %%zmm16\n\tvpternlogd $0xff, %%zmm17, %%zmm17, %%zmm17\n\t"
        "vpternlogd $0xff, %%zmm18, %%zmm18, %%zmm18\n\tvpternlogd $0xff, %%zmm19, %%zmm19, %%zmm19\n\t"
        "vpternlogd $0xff, %%zmm20, %%zmm20, %%zmm20\n\tvpternlogd $0xff, %%zmm21, %%zmm21, %%zmm21\n\t"
        "vpternlogd $0xff, %%zmm22, %%zmm22, %%zmm22\n\tvpternlogd $0xff, %%zmm23, %%zmm23, %%zmm23\n\t"
        "vpternlogd $0xff, %%zmm24, %%zmm24, %%zmm24\n\tvpternlogd $0xff, %%zmm25, %%zmm25, %%zmm25\n\t"
        "vpternlogd $0xff, %%zmm26, %%zmm26, %%zmm26\n\tvpternlogd $0xff, %%zmm27, %%zmm27, %%zmm27\n\t"
        "vpternlogd $0xff, %%zmm28, %%zmm28, %%zmm28\n\tvpternlogd $0xff, %%zmm29, %%zmm29, %%zmm29\n\t"
        "vpternlogd $0xff, %%zmm30, %%zmm30, %%zmm30\n\tvpternlogd $0xff, %%zmm31, %%zmm31, %%zmm31\n\t"
        "vpternlogd $0xff, %%zmm0, %%zmm0, %%zmm0\n\tvpternlogd $0xff, %%zmm1, %%zmm1, %%zmm1\n\t"
        "vpternlogd $0xff, %%zmm2, %%zmm2, %%zmm2\n\tvpternlogd $0xff, %%zmm3, %%zmm3, %%zmm3\n\t"
        "vpternlogd $0xff, %%zmm4, %%zmm4, %%zmm4\n\tvpternlogd $0xff, %%zmm5, %%zmm5, %%zmm5\n\t"
        "vpternlogd $0xff, %%zmm6, %%zmm6, %%zmm6\n\tvpternlogd $0xff, %%zmm7, %%zmm7, %%zmm7\n\t"
        "vzeroupper"
        : : : "xmm0", "xmm1", "xmm2", "xmm3", "xmm4", "xmm5", "xmm6", "xmm7",
              "xmm16", "xmm17", "xmm18", "xmm19", "xmm20", "xmm21", "xmm22", "xmm23",
              "xmm24", "xmm25", "xmm26", "xmm27", "xmm28", "xmm29", "xmm30", "xmm31");
}

/* One instruction per operation. */
NOINLINE static void chain(int *out, const int *x, const int *y)
{
    __m512i a = load(x), b = load(y);
    __m512i s = _mm512_add_epi32(a, b);
    __m512i m = _mm512_mullo_epi32(s, b);
    __m512i d = _mm512_sub_epi32(b, a);
    store(out, _mm512_xor_si512(m, d));
}

/* A copy keeps the value its source had. */
NOINLINE static void copy_then_change(int *out, const int *x, const int *y)
{
    __m512i a = load(x), b = load(y);
    __m512i t = a;
    a = _mm512_add_epi32(a, b);
    __m512i u = a;
    a = _mm512_add_epi32(a, a);
    store(out, _mm512_sub_epi32(_mm512_xor_si512(t, a), u));
}

/* A value carried around a loop. */
NOINLINE static void accumulate(int *out, const int *x, long n)
{
    __m512i acc = _mm512_setzero_si512();
    for (long i = 0; i < n; i++) {
        __m512i t = load(x + 16 * (i & 3));
        acc = _mm512_add_epi32(_mm512_slli_epi32(acc, 1), t);
    }
    store(out, acc);
}

/* Written on both arms while another value lives across the join. */
NOINLINE static void join(int *out, const int *x, const int *y, int k)
{
    __m512i a = load(x), b = load(y), r;
    if (k & 1)
        r = _mm512_add_epi32(a, b);
    else
        r = _mm512_sub_epi32(a, b);
    if (k & 2)
        a = _mm512_slli_epi32(a, 2);
    store(out, _mm512_xor_si512(r, a));
}

/* A cycle of copies: each of the three names takes the next one's value. */
NOINLINE static void rotate(int *out, const int *x, long n)
{
    __m512i a = load(x), b = load(x + 16), c = load(x + 32);
    for (long i = 0; i < n; i++) {
        __m512i t = a;
        a = b;
        b = c;
        c = _mm512_add_epi32(t, b);
    }
    store(out, a);
    store(out + 16, b);
    store(out + 32, c);
}

/* Twenty values alive at once: more than there are registers. */
NOINLINE static void twenty(int *out, const int *x)
{
    __m512i base = load(x);
#define V(i) __m512i v##i = _mm512_add_epi32(base, _mm512_set1_epi32(3 * i + 1));
    V(0) V(1) V(2) V(3) V(4) V(5) V(6) V(7) V(8) V(9)
    V(10) V(11) V(12) V(13) V(14) V(15) V(16) V(17) V(18) V(19)
#undef V
    __m512i acc = v19;
#define SUB(i) acc = _mm512_sub_epi32(acc, v##i);
#define XOR(i) acc = _mm512_xor_si512(acc, v##i);
    SUB(18) XOR(17) SUB(16) XOR(15) SUB(14) XOR(13) SUB(12) XOR(11) SUB(10) XOR(9)
    SUB(8) XOR(7) SUB(6) XOR(5) SUB(4) XOR(3) SUB(2) XOR(1) SUB(0)
#undef SUB
#undef XOR
    store(out, acc);
}

/* Two values that live across a call nobody saves registers for. */
NOINLINE static void across_call(int *out, const int *x)
{
    __m512i a = load(x);
    __m512i b = _mm512_add_epi32(a, a);
    scribble();
    store(out, _mm512_xor_si512(_mm512_slli_epi32(a, 3), b));
}

/* Values on either side of a call, none across it. */
NOINLINE static void around_call(int *out, const int *x, const int *y)
{
    __m512i a = load(x);
    store(out, _mm512_add_epi32(a, a));
    scribble();
    __m512i c = load(y);
    store(out + 16, _mm512_sub_epi32(c, _mm512_set1_epi32(7)));
}

/* A call in a loop: the accumulator crosses it, the temporary does not. */
NOINLINE static void call_in_loop(int *out, const int *x, long n)
{
    __m512i acc = load(x);
    for (long i = 0; i < n; i++) {
        __m512i t = _mm512_mullo_epi32(load(x + 16), _mm512_set1_epi32((int)i + 2));
        acc = _mm512_xor_si512(acc, t);
        scribble();
    }
    store(out, acc);
}

/* The result of a three-source operation replaces each source in turn. */
NOINLINE static void ternary(int *out, const int *x, const int *y, const int *z)
{
    __m512i a = load(x), b = load(y), c = load(z);
    __m512i r = _mm512_ternarylogic_epi32(a, b, c, 0x96);
    store(out, r);
    __m512i a2 = a, b2 = b, c2 = c;
    a2 = _mm512_ternarylogic_epi32(a2, b, c, 0xca);
    b2 = _mm512_ternarylogic_epi32(a, b2, c, 0xca);
    c2 = _mm512_ternarylogic_epi32(a, b, c2, 0xca);
    store(out + 16, a2);
    store(out + 32, b2);
    store(out + 48, c2);
    /* One object in all three places, and the result back in it. */
    a = _mm512_ternarylogic_epi32(a, a, a, 0x0f);
    store(out + 64, a);
}

/* The same for the fused multiply-add forms. */
NOINLINE static void fused(float *out, const float *x, const float *y, const float *z)
{
    __m512 a = _mm512_loadu_ps(x), b = _mm512_loadu_ps(y), c = _mm512_loadu_ps(z);
    __m512 a2 = a, b2 = b, c2 = c;
    a2 = _mm512_fmadd_ps(a2, b, c);
    b2 = _mm512_fmsub_ps(a, b2, c);
    c2 = _mm512_fnmadd_ps(a, b, c2);
    _mm512_storeu_ps(out, a2);
    _mm512_storeu_ps(out + 16, b2);
    _mm512_storeu_ps(out + 32, c2);
    _mm512_storeu_ps(out + 48, _mm512_fnmsub_ps(a, b, c));
}

/* A compare writes a mask; its operands stay what they were. */
NOINLINE static unsigned masks(int *out, const int *x, const int *y)
{
    __m512i a = load(x), b = load(y);
    unsigned equal = _mm512_cmpeq_epi32_mask(_mm512_and_si512(a, _mm512_set1_epi32(3)),
                                             _mm512_and_si512(b, _mm512_set1_epi32(3)));
    unsigned less = _mm512_cmp_epi32_mask(a, b, _MM_CMPINT_LT);
    store(out, _mm512_add_epi32(a, b));
    return equal | less << 16;
}

/* A shift by a count that is not a constant, past the lane as well. */
NOINLINE static void shifts(int *out, const int *x, unsigned n)
{
    __m512i a = load(x);
    store(out, _mm512_slli_epi32(a, n));
    store(out + 16, _mm512_srai_epi32(a, n));
    a = _mm512_srli_epi32(a, n);
    store(out + 32, a);
}

/* An object whose address is taken half way through is memory. */
NOINLINE static void poke(__m512i *p, int lane)
{
    int lanes[16];
    memcpy(lanes, p, sizeof lanes);
    lanes[lane] = -lanes[lane];
    memcpy(p, lanes, sizeof lanes);
}

NOINLINE static void address_taken(int *out, const int *x, const int *y)
{
    __m512i a = load(x), b = load(y);
    a = _mm512_add_epi32(a, b);
    poke(&a, 5);
    b = _mm512_sub_epi32(b, a);
    store(out, _mm512_add_epi32(a, b));
    store(out + 16, b);
}

/* A part of an object is read: the half, a quarter, a lane. */
NOINLINE static void parts(int *out, const int *x, const int *y)
{
    __m512i a = _mm512_add_epi32(load(x), load(y));
    __m256i low = _mm512_castsi512_si256(a);
    __m256i high = _mm512_extracti64x4_epi64(a, 1);
    _mm256_storeu_si256((__m256i *)out, _mm256_add_epi32(low, high));
    int lanes[16];
    store(lanes, a);
    out[8] = lanes[3] - lanes[12];
}

/* By value into a function and back, called and inlined. */
NOINLINE static __m512i mix(__m512i a, __m512i b)
{
    return _mm512_sub_epi32(_mm512_xor_si512(a, b), _mm512_srli_epi32(b, 5));
}

static inline __m512i choose(__m512i a, __m512i b, __m512i mask)
{
    __m512i t = _mm512_and_si512(a, mask);
    __m512i u = _mm512_andnot_si512(mask, b);
    return _mm512_or_si512(t, u);
}

NOINLINE static void by_value(int *out, const int *x, const int *y)
{
    __m512i a = load(x), b = load(y);
    __m512i r = mix(a, b);
    r = choose(r, a, b);
    r = mix(r, choose(b, r, a));
    store(out, r);
}

/* 64 bytes that are no vector, copied whole beside vector code. */
struct blob { unsigned char bytes[64]; };

NOINLINE static void blob_beside_vectors(struct blob *out, const struct blob *in, int *vout,
                                         const int *vin)
{
    struct blob t = *in;
    __m512i v = load(vin);
    v = _mm512_add_epi32(v, v);
    struct blob u = t;
    *out = u;
    store(vout, v);
}

/* The same copies in a function that has no vector code. */
NOINLINE static void blob_alone(struct blob *out, const struct blob *in)
{
    struct blob t = *in;
    struct blob u = t;
    *out = u;
}

/* Branches, a loop and an early exit together. */
NOINLINE static void tangle(int *out, const int *x, long n, long stop)
{
    __m512i acc = load(x), other = load(x + 16);
    for (long i = 0; i < n; i++) {
        __m512i t = load(x + 16 * (i & 1));
        if (i & 2) {
            acc = _mm512_xor_si512(acc, t);
        } else {
            __m512i keep = acc;
            acc = _mm512_add_epi32(other, t);
            other = keep;
        }
        if (i == stop)
            break;
        switch (i % 3) {
        case 0: other = _mm512_sub_epi32(other, acc); break;
        case 1: acc = _mm512_max_epi32(acc, other); break;
        default: break;
        }
    }
    store(out, acc);
    store(out + 16, other);
}

int main(void)
{
    if (!__builtin_cpu_supports("avx512f") || !__builtin_cpu_supports("avx512bw")
        || !__builtin_cpu_supports("avx512dq")) {
        puts("zmm homes: no AVX-512 F, BW and DQ on this machine");
        return 0;
    }

    int x[64], y[64], z[64], got[96], want[96];
    for (int i = 0; i < 64; i++) {
        x[i] = 1000003 * i + 17;
        y[i] = 77 - 40503 * i * i;
        z[i] = (i * 2654435761u) >> 3;
    }

    chain(got, x, y);
    for (int i = 0; i < 16; i++)
        want[i] = (int)((unsigned)(x[i] + y[i]) * (unsigned)y[i]) ^ (y[i] - x[i]);
    expect("chain", got, want, 64);

    copy_then_change(got, x, y);
    for (int i = 0; i < 16; i++) {
        unsigned a = x[i], b = y[i], t = a;
        a += b;
        unsigned u = a;
        a += a;
        want[i] = (int)((t ^ a) - u);
    }
    expect("copy_then_change", got, want, 64);

    for (long n = 0; n < 7; n++) {
        accumulate(got, x, n);
        for (int i = 0; i < 16; i++) {
            unsigned acc = 0;
            for (long k = 0; k < n; k++)
                acc = (acc << 1) + (unsigned)x[16 * (k & 3) + i];
            want[i] = (int)acc;
        }
        expect("accumulate", got, want, 64);
    }

    for (int k = 0; k < 4; k++) {
        join(got, x, y, k);
        for (int i = 0; i < 16; i++) {
            unsigned a = x[i], b = y[i];
            unsigned r = k & 1 ? a + b : a - b;
            if (k & 2)
                a <<= 2;
            want[i] = (int)(r ^ a);
        }
        expect("join", got, want, 64);
    }

    for (long n = 0; n < 6; n++) {
        rotate(got, x, n);
        for (int i = 0; i < 16; i++) {
            unsigned a = x[i], b = x[16 + i], c = x[32 + i];
            for (long k = 0; k < n; k++) {
                unsigned t = a;
                a = b;
                b = c;
                c = t + b;
            }
            want[i] = (int)a;
            want[16 + i] = (int)b;
            want[32 + i] = (int)c;
        }
        expect("rotate", got, want, 192);
    }

    twenty(got, x);
    for (int i = 0; i < 16; i++) {
        unsigned acc = (unsigned)x[i] + 3 * 19 + 1;
        for (int k = 18; k >= 0; k--) {
            unsigned v = (unsigned)x[i] + 3 * k + 1;
            acc = k % 2 == 0 ? acc - v : acc ^ v;
        }
        want[i] = (int)acc;
    }
    expect("twenty", got, want, 64);

    across_call(got, x);
    for (int i = 0; i < 16; i++)
        want[i] = (int)(((unsigned)x[i] << 3) ^ ((unsigned)x[i] * 2));
    expect("across_call", got, want, 64);

    around_call(got, x, y);
    for (int i = 0; i < 16; i++) {
        want[i] = (int)((unsigned)x[i] * 2);
        want[16 + i] = (int)((unsigned)y[i] - 7);
    }
    expect("around_call", got, want, 128);

    for (long n = 0; n < 5; n++) {
        call_in_loop(got, x, n);
        for (int i = 0; i < 16; i++) {
            unsigned acc = x[i];
            for (long k = 0; k < n; k++)
                acc ^= (unsigned)x[16 + i] * (unsigned)(k + 2);
            want[i] = (int)acc;
        }
        expect("call_in_loop", got, want, 64);
    }

    ternary(got, x, y, z);
    for (int i = 0; i < 16; i++) {
        unsigned a = x[i], b = y[i], c = z[i];
        unsigned select = (a & b) | (~a & c);
        want[i] = (int)(a ^ b ^ c);
        want[16 + i] = (int)select;
        want[32 + i] = (int)select;
        want[48 + i] = (int)select;
        want[64 + i] = (int)~a;
    }
    expect("ternary", got, want, 320);

    {
        float fx[16], fy[16], fz[16], fgot[64], fwant[64];
        for (int i = 0; i < 16; i++) {
            fx[i] = (float)(i - 7);
            fy[i] = (float)(3 * i + 1);
            fz[i] = (float)(100 - 9 * i);
        }
        fused(fgot, fx, fy, fz);
        for (int i = 0; i < 16; i++) {
            /* Small integers: every product and sum is exact. */
            fwant[i] = fx[i] * fy[i] + fz[i];
            fwant[16 + i] = fx[i] * fy[i] - fz[i];
            fwant[32 + i] = -(fx[i] * fy[i]) + fz[i];
            fwant[48 + i] = -(fx[i] * fy[i]) - fz[i];
        }
        expect("fused", fgot, fwant, sizeof fwant);
    }

    {
        unsigned m = masks(got, x, y), equal = 0, less = 0;
        for (int i = 0; i < 16; i++) {
            equal |= (unsigned)((x[i] & 3) == (y[i] & 3)) << i;
            less |= (unsigned)(x[i] < y[i]) << i;
            want[i] = (int)((unsigned)x[i] + (unsigned)y[i]);
        }
        unsigned want_m = equal | less << 16;
        expect("masks", &m, &want_m, sizeof m);
        expect("masks operands", got, want, 64);
    }

    {
        static const unsigned counts[] = { 0, 1, 5, 31, 32, 40, 255, 256 };
        for (unsigned k = 0; k < sizeof counts / sizeof counts[0]; k++) {
            unsigned n = counts[k];
            shifts(got, x, n);
            for (int i = 0; i < 16; i++) {
                want[i] = n > 31 ? 0 : (int)((unsigned)x[i] << n);
                want[16 + i] = x[i] >> (n > 31 ? 31 : n);
                want[32 + i] = n > 31 ? 0 : (int)((unsigned)x[i] >> n);
            }
            expect("shifts", got, want, 192);
        }
    }

    address_taken(got, x, y);
    for (int i = 0; i < 16; i++) {
        unsigned a = (unsigned)x[i] + (unsigned)y[i];
        if (i == 5)
            a = -a;
        unsigned b = (unsigned)y[i] - a;
        want[i] = (int)(a + b);
        want[16 + i] = (int)b;
    }
    expect("address_taken", got, want, 128);

    parts(got, x, y);
    for (int i = 0; i < 8; i++)
        want[i] = (int)((unsigned)x[i] + (unsigned)y[i] + (unsigned)x[8 + i] + (unsigned)y[8 + i]);
    want[8] = (int)(((unsigned)x[3] + (unsigned)y[3]) - ((unsigned)x[12] + (unsigned)y[12]));
    expect("parts", got, want, 36);

    by_value(got, x, y);
    for (int i = 0; i < 16; i++) {
        unsigned a = x[i], b = y[i];
        unsigned r = (a ^ b) - (b >> 5);
        r = (r & b) | (~b & a);
        unsigned other = (b & a) | (~a & r);
        want[i] = (int)((r ^ other) - (other >> 5));
    }
    expect("by_value", got, want, 64);

    {
        struct blob in, out, alone;
        for (int i = 0; i < 64; i++)
            in.bytes[i] = (unsigned char)(i * 37 + 11);
        memset(&out, 0, sizeof out);
        memset(&alone, 0, sizeof alone);
        blob_beside_vectors(&out, &in, got, x);
        blob_alone(&alone, &in);
        for (int i = 0; i < 16; i++)
            want[i] = (int)((unsigned)x[i] * 2);
        expect("blob_beside_vectors", &out, &in, sizeof in);
        expect("blob_beside_vectors vector", got, want, 64);
        expect("blob_alone", &alone, &in, sizeof in);
    }

    for (long n = 0; n < 9; n++) {
        for (long stop = -1; stop < 9; stop += 3) {
            tangle(got, x, n, stop);
            for (int i = 0; i < 16; i++) {
                unsigned acc = x[i], other = x[16 + i];
                for (long k = 0; k < n; k++) {
                    unsigned t = x[16 * (k & 1) + i];
                    if (k & 2) {
                        acc ^= t;
                    } else {
                        unsigned keep = acc;
                        acc = other + t;
                        other = keep;
                    }
                    if (k == stop)
                        break;
                    if (k % 3 == 0)
                        other -= acc;
                    else if (k % 3 == 1)
                        acc = (int)acc > (int)other ? acc : other;
                }
                want[i] = (int)acc;
                want[16 + i] = (int)other;
            }
            expect("tangle", got, want, 128);
        }
    }

    if (failures == 0)
        puts("zmm homes: ok");
    return failures != 0;
}
