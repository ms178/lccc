/* Regression: the MMX 64-bit scalar moves, integer add/sub/mul, whole-register
 * shifts and packed shifts by a scalar count (PSLL/PSRL/PSRA on w/d/q).
 *
 * Every result is printed as its 64 raw bits. The output must match GCC 16.2
 * byte for byte at -O0 and -O2. Counts cover 0, the element width minus one,
 * the width itself, and far past it. Logical shifts give zero past the width;
 * arithmetic shifts fill with the sign bit. Operands have their sign bits and
 * their high halves set so the wrap-around and the sign handling are exercised. */
#include <emmintrin.h>  /* _mm_mul_su32 is declared here, as in GCC 16.2 */
#include <stdio.h>

/* GCC 16.2 names for the 64-bit scalar moves. Clang 23.1.5's mmintrin.h does
 * not declare these three, so under __clang__ the same bit pattern is built
 * with a vector cast. The arithmetic under test is unchanged. */
#ifdef __clang__
#define MK64(x) ((__m64)(long long)(x))
#define FROM64(v) ((long long)(v))
#define CVTI64X(x) ((__m64)(long long)(x))
#else
#define MK64(x) _mm_set_pi64x(x)
#define FROM64(v) _mm_cvtsi64_si64x(v)
#define CVTI64X(x) _mm_cvtsi64x_si64(x)
#endif

static unsigned long long bits(__m64 v)
{
    return (unsigned long long)_mm_cvtm64_si64(v);
}

static void show(const char *tag, __m64 v)
{
    printf("%-22s %016llx\n", tag, bits(v));
}

int main(void)
{
    __m64 a = MK64((long long)0xF0E1D2C3B4A59687ULL);
    __m64 b = MK64((long long)0x7FFFFFFF80000001ULL);
    __m64 lo = MK64(0x00000000FFFFFFFFLL);
    __m64 big = MK64((long long)0x8000000000000000ULL);
    __m64 neg = MK64((long long)0xFFFEFFFF80018000ULL);

    show("cvtsi64x roundtrip", CVTI64X((long long)0x0123456789ABCDEFLL));
    printf("cvtsi64_si64x         %016llx\n", (unsigned long long)FROM64(a));
    show("set_pi64x", b);
    show("add_si64 wrap", _mm_add_si64(a, b));
    show("add_si64 carry", _mm_add_si64(lo, lo));
    show("sub_si64 borrow", _mm_sub_si64(b, a));
    show("sub_si64 big", _mm_sub_si64(big, lo));
    show("mul_su32 low dwords", _mm_mul_su32(a, b));
    show("mul_su32 max", _mm_mul_su32(lo, lo));

    static const long long counts[] = { 0, 1, 15, 16, 31, 32, 63, 64, 200 };
    for (unsigned i = 0; i < sizeof counts / sizeof counts[0]; i++) {
        __m64 c = MK64(counts[i]);
        char tag[64];
        snprintf(tag, sizeof tag, "sll_si64 %lld", counts[i]);
        show(tag, _mm_sll_si64(a, c));
        snprintf(tag, sizeof tag, "srl_si64 %lld", counts[i]);
        show(tag, _mm_srl_si64(a, c));
        snprintf(tag, sizeof tag, "sll_pi16 %lld", counts[i]);
        show(tag, _mm_sll_pi16(a, c));
        snprintf(tag, sizeof tag, "srl_pi16 %lld", counts[i]);
        show(tag, _mm_srl_pi16(a, c));
        snprintf(tag, sizeof tag, "sra_pi16 %lld", counts[i]);
        show(tag, _mm_sra_pi16(neg, c));
        snprintf(tag, sizeof tag, "sll_pi32 %lld", counts[i]);
        show(tag, _mm_sll_pi32(a, c));
        snprintf(tag, sizeof tag, "srl_pi32 %lld", counts[i]);
        show(tag, _mm_srl_pi32(a, c));
        snprintf(tag, sizeof tag, "sra_pi32 %lld", counts[i]);
        show(tag, _mm_sra_pi32(neg, c));
    }
    return 0;
}
