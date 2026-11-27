/* `asm` operands in `zmm` and mask registers: the classes "v" and "x" for a
 * 512-bit vector, "k" and "Yk" for an integer.
 *
 * lccc loaded eight bytes of a 512-bit operand into an `xmm` register
 * (wrong values, no diagnostic), refused "k" as an impossible constraint
 * and gave "Yk" an `xmm` register, which stopped in the assembler.  The
 * expected values are computed in C.
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

static void expect_mask(const char *what, unsigned long long got, unsigned long long want)
{
    if (got != want) {
        printf("FAIL %s: %llx, expected %llx\n", what, got, want);
        failures++;
    }
}

static __m512i load(const int *p) { return _mm512_loadu_si512(p); }
static void store(int *p, __m512i v) { _mm512_storeu_si512(p, v); }

NOINLINE static void add(int *out, const int *a, const int *b)
{
    __m512i va = load(a), vb = load(b), r;
    __asm__("vpaddd %2, %1, %0" : "=v"(r) : "v"(va), "x"(vb));
    store(out, r);
}

/* A write mask that zeroes. */
NOINLINE static void add_maskz(int *out, const int *a, const int *b, unsigned short m)
{
    __m512i va = load(a), vb = load(b), r;
    __asm__("vpaddd %2, %1, %0%{%3%}%{z%}" : "=v"(r) : "v"(va), "v"(vb), "Yk"(m));
    store(out, r);
}

/* A write mask that merges into an input tied to the output. */
NOINLINE static void add_mask(int *out, const int *src, const int *a, const int *b,
                              unsigned short m)
{
    __m512i vs = load(src), va = load(a), vb = load(b), r;
    __asm__("vpaddd %3, %2, %0%{%4%}" : "=v"(r) : "0"(vs), "v"(va), "v"(vb), "Yk"(m));
    store(out, r);
}

/* A mask that is no lvalue, and eight bits for eight lanes. */
NOINLINE static void add_quads(long long *out, const long long *a, const long long *b, int bits)
{
    __m512i va = _mm512_loadu_si512(a), vb = _mm512_loadu_si512(b), r;
    __asm__("vpaddq %2, %1, %0%{%3%}%{z%}"
            : "=v"(r) : "v"(va), "v"(vb), "Yk"((unsigned char)(bits >> 4)));
    _mm512_storeu_si512(out, r);
}

/* A compare writes a mask. */
NOINLINE static unsigned short greater(const int *a, const int *b)
{
    __m512i va = load(a), vb = load(b);
    unsigned short m;
    __asm__("vpcmpgtd %2, %1, %0" : "=k"(m) : "v"(va), "v"(vb));
    return m;
}

/* Masks only: three inputs, an early-clobbered output. */
NOINLINE static unsigned short mask_logic(unsigned short a, unsigned short b, unsigned short c)
{
    unsigned short r;
    __asm__("kandw %2, %1, %0\n\tkxorw %3, %0, %0" : "=&k"(r) : "k"(a), "k"(b), "k"(c));
    return r;
}

/* A read-write mask, and one tied to the output. */
NOINLINE static unsigned short mask_shift(unsigned short m)
{
    __asm__("kshiftlw $3, %0, %0" : "+k"(m));
    return m;
}

NOINLINE static unsigned short mask_tied(unsigned short a, unsigned short b)
{
    unsigned short r;
    __asm__("korw %2, %0, %0" : "=k"(r) : "0"(a), "k"(b));
    return r;
}

/* The `ymm` and `xmm` views of a `zmm` operand. */
NOINLINE static void upper_half(int *out, const int *a)
{
    __m512i va = load(a), r;
    __asm__("vextracti64x4 $1, %1, %t0\n\tvpaddd %x1, %x0, %x0" : "=&x"(r) : "x"(va));
    store(out, r);
}

NOINLINE static void floats(float *out, const float *a, const float *b)
{
    float twos[16];
    for (int i = 0; i < 16; i++) twos[i] = 2.0f;
    __m512 va = _mm512_loadu_ps(a), vb = _mm512_loadu_ps(b), acc = _mm512_loadu_ps(twos);
    __asm__("vmulps %2, %1, %%zmm0\n\tvaddps %%zmm0, %0, %0"
            : "+v"(acc) : "v"(va), "v"(vb) : "xmm0");
    _mm512_storeu_ps(out, acc);
}

/* AVX-512BW: thirty-two and sixty-four bits of mask. */
NOINLINE static unsigned equal_words(const short *a, const short *b)
{
    __m512i va = _mm512_loadu_si512(a), vb = _mm512_loadu_si512(b);
    unsigned m;
    __asm__("vpcmpeqw %2, %1, %0" : "=k"(m) : "v"(va), "v"(vb));
    return m;
}

NOINLINE static unsigned long long equal_bytes(const char *a, const char *b)
{
    __m512i va = _mm512_loadu_si512(a), vb = _mm512_loadu_si512(b);
    unsigned long long m;
    __asm__("vpcmpeqb %2, %1, %0" : "=k"(m) : "v"(va), "v"(vb));
    return m;
}

NOINLINE static void keep_bytes(char *out, const char *a, unsigned long long m)
{
    __m512i va = _mm512_loadu_si512(a), r;
    __asm__("vmovdqu8 %1, %0%{%2%}%{z%}" : "=v"(r) : "v"(va), "Yk"(m));
    _mm512_storeu_si512(out, r);
}

int main(void)
{
    if (!__builtin_cpu_supports("avx512f") || !__builtin_cpu_supports("avx512bw")) {
        puts("asm AVX-512 operands: no AVX-512F and BW on this machine");
        return 0;
    }

    int a[16], b[16], src[16], got[16], want[16];
    for (int i = 0; i < 16; i++) {
        a[i] = 500 + 11 * i;
        b[i] = 9 - 2 * i * i;
        src[i] = -1000 - i;
    }

    add(got, a, b);
    for (int i = 0; i < 16; i++) want[i] = a[i] + b[i];
    expect("add", got, want, sizeof want);

    add_maskz(got, a, b, 0xA5C3);
    for (int i = 0; i < 16; i++) want[i] = (0xA5C3 >> i) & 1 ? a[i] + b[i] : 0;
    expect("add_maskz", got, want, sizeof want);

    add_mask(got, src, a, b, 0x3C5A);
    for (int i = 0; i < 16; i++) want[i] = (0x3C5A >> i) & 1 ? a[i] + b[i] : src[i];
    expect("add_mask", got, want, sizeof want);

    {
        long long qa[8], qb[8], qgot[8], qwant[8];
        int bits = 0x7F5B6;
        for (int i = 0; i < 8; i++) {
            qa[i] = 0x100000000LL * (i + 1);
            qb[i] = 17 - i;
            qwant[i] = ((bits >> 4) >> i) & 1 ? qa[i] + qb[i] : 0;
        }
        add_quads(qgot, qa, qb, bits);
        expect("add_quads", qgot, qwant, sizeof qwant);
    }

    {
        unsigned short want_mask = 0;
        for (int i = 0; i < 16; i++) want_mask |= (unsigned short)((a[i] > -b[i] * 3) << i);
        int scaled[16];
        for (int i = 0; i < 16; i++) scaled[i] = -b[i] * 3;
        expect_mask("greater", greater(a, scaled), want_mask);
    }

    expect_mask("mask_logic", mask_logic(0xF0F0, 0x3C3C, 0x00FF), (0xF0F0 & 0x3C3C) ^ 0x00FF);
    expect_mask("mask_shift", mask_shift(0x9005), (unsigned short)(0x9005 << 3));
    expect_mask("mask_tied", mask_tied(0x1200, 0x0034), 0x1234);

    upper_half(got, a);
    for (int i = 0; i < 4; i++) want[i] = a[i + 8] + a[i];
    for (int i = 4; i < 16; i++) want[i] = 0;
    expect("upper_half", got, want, sizeof want);

    {
        float fa[16], fb[16], fgot[16], fwant[16];
        for (int i = 0; i < 16; i++) {
            fa[i] = (float)(i - 5);
            fb[i] = 0.25f * (float)i;
            fwant[i] = 2.0f + fa[i] * fb[i];
        }
        floats(fgot, fa, fb);
        expect("floats", fgot, fwant, sizeof fwant);
    }

    {
        short wa[32], wb[32];
        unsigned want_mask = 0;
        for (int i = 0; i < 32; i++) {
            wa[i] = (short)(i * 3);
            wb[i] = (short)(i % 3 == 0 ? i * 3 : -i);
            want_mask |= (unsigned)(wa[i] == wb[i]) << i;
        }
        expect_mask("equal_words", equal_words(wa, wb), want_mask);
    }

    {
        char ca[64], cb[64], cgot[64], cwant[64];
        unsigned long long want_mask = 0, keep = 0x8000F0F012345679ULL;
        for (int i = 0; i < 64; i++) {
            ca[i] = (char)(i + 1);
            cb[i] = (char)(i % 5 == 0 || i > 60 ? i + 1 : 0);
            want_mask |= (unsigned long long)(ca[i] == cb[i]) << i;
            cwant[i] = (keep >> i) & 1 ? ca[i] : 0;
        }
        expect_mask("equal_bytes", equal_bytes(ca, cb), want_mask);
        keep_bytes(cgot, ca, keep);
        expect("keep_bytes", cgot, cwant, sizeof cwant);
    }

    if (failures == 0)
        puts("asm AVX-512 operands: PASS");
    return failures != 0;
}
