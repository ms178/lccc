// FLAGS: -msse4.2 -mavx2
/* A constant argument is converted to its parameter's type once.
 *
 * An intrinsic's scalar parameter is an `int`, a `char`, a `long long`, a
 * `float`: C converts the argument to it as an assignment would, whatever
 * type the argument was written in. For a constant the lowering makes that
 * conversion itself, so that an immediate stays a constant without the
 * optimizer. A floating constant given to an integer parameter was converted
 * twice: read as the integer it becomes (3.7 is 3), and then converted "from
 * double" again, with the integer's bits taken for a double. The optimizer
 * folded the second conversion by value and nothing showed; without it the
 * code generator converted the bits, and `_mm_set1_epi32(3.7)` was a vector
 * of zeros, `_mm_slli_epi32(v, 2.9)` a shift by none, `_mm_crc32_u32(1.5,
 * 77.7)` the checksum of two zeros.
 *
 * The rows: floating constants for integer parameters of every width (the
 * ones that fit an `int` and the ones only a `long long` holds), integer
 * constants wider or narrower than their parameter, signed for unsigned and
 * the reverse, and floating parameters given constants of either kind (the
 * path that must not take the integer reading). This suite runs each case
 * with and without optimization, against GCC.
 */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

#define KEEP __attribute__((noinline)) static

static void row(const char *what, const void *bytes, int count)
{
    unsigned u[8] = { 0 };
    memcpy(u, bytes, (size_t)count);
    printf("%-18s", what);
    for (int i = 0; i < count / 4; i++)
        printf(" %08x", u[i]);
    printf("\n");
}
#define ROW(name, type, expr) do { type r_ = (expr); row(name, &r_, (int)sizeof r_); } while (0)

/* ---- floating constants, integer parameters ---- */
KEEP __m128i set1_epi32_double(void) { return _mm_set1_epi32(3.7); }
KEEP __m128i set1_epi16_float(void) { return _mm_set1_epi16(3.7f); }
KEEP __m128i set1_epi8_negative(void) { return _mm_set1_epi8(-3.7); }
KEEP __m128i set1_epi64x_double(void) { return _mm_set1_epi64x(3.7); }
KEEP __m128i set1_epi64x_wide(void) { return _mm_set1_epi64x(3e10); }
KEEP __m128i set_epi32_mixed(void) { return _mm_set_epi32(1.5, 2.5f, 3.9, -4.9); }
KEEP __m128i set_epi16_mixed(void) { return _mm_set_epi16(1.5, 2, 3.5, 4, 5.5, 6, 7.5, 8); }
KEEP __m128i cvtsi32_si128_double(void) { return _mm_cvtsi32_si128(9.9); }
KEEP __m128i cvtsi64_si128_wide(void) { return _mm_cvtsi64_si128(9.9e10); }
KEEP __m128i insert_epi32_double(__m128i v) { return _mm_insert_epi32(v, 7.7, 1); }
KEEP __m128i insert_epi16_float(__m128i v) { return _mm_insert_epi16(v, 7.7f, 2); }
KEEP __m128i insert_epi8_double(__m128i v) { return _mm_insert_epi8(v, 7.7, 3); }
KEEP __m128i insert_epi64_double(__m128i v) { return _mm_insert_epi64(v, 7.7, 1); }
KEEP __m256i set1_epi8_256(void) { return _mm256_set1_epi8(65.5); }
KEEP __m256i set1_epi16_256(void) { return _mm256_set1_epi16(-2.5f); }
KEEP __m256i set1_epi32_256(void) { return _mm256_set1_epi32(3.7); }
KEEP __m256i set1_epi64x_256(void) { return _mm256_set1_epi64x(3e10); }
KEEP __m256i set_epi32_256(void) { return _mm256_set_epi32(1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5, 8.5); }
KEEP __m256i set_epi64x_256(void) { return _mm256_set_epi64x(1.5, 2.5, 3.5, 4e12); }
KEEP __m128 cvtsi32_ss_double(__m128 v) { return _mm_cvtsi32_ss(v, 5.5); }
KEEP __m128d cvtsi32_sd_float(__m128d v) { return _mm_cvtsi32_sd(v, 6.5f); }
KEEP __m128d cvtsi64_sd_wide(__m128d v) { return _mm_cvtsi64_sd(v, 6.5e10); }
KEEP unsigned crc32_u32_double(void) { return _mm_crc32_u32(1.5, 77.7); }
KEEP unsigned crc32_u8_double(void) { return _mm_crc32_u8(100.5, 7.7); }
KEEP __m128i slli_epi32_double(__m128i v) { return _mm_slli_epi32(v, 2.9); }
KEEP __m128i srai_epi16_float(__m128i v) { return _mm_srai_epi16(v, 1.5f); }
/* the same constants inside an operation, where the splat is an operand */
KEEP __m256i add_set1_double(__m256i v) { return _mm256_add_epi32(v, _mm256_set1_epi32(3.7)); }
KEEP __m256i add_set1_float(__m256i v) { return _mm256_add_epi32(v, _mm256_set1_epi32(-2.9f)); }
KEEP __m128i add_set1_128(__m128i v) { return _mm_add_epi32(v, _mm_set1_epi32(3.7)); }
KEEP __m256i add_set1_epi16(__m256i v) { return _mm256_add_epi16(v, _mm256_set1_epi16(3.7)); }
KEEP __m256i add_set1_epi64x(__m256i v) { return _mm256_add_epi64(v, _mm256_set1_epi64x(3.7)); }
KEEP __m256i add_set1_epi8(__m256i v) { return _mm256_add_epi8(v, _mm256_set1_epi8(3.7)); }

/* ---- integer constants of another width or signedness ---- */
KEEP __m128i set1_epi32_bit31(void) { return _mm_set1_epi32(1u << 31); }
KEEP __m128i set1_epi32_33bits(void) { return _mm_set1_epi32(0x1ffffffffll); }
KEEP __m128i set1_epi8_511(void) { return _mm_set1_epi8(511); }
KEEP __m128i set1_epi16_minus1ll(void) { return _mm_set1_epi16(-1ll); }
KEEP __m128i set_epi64x_mixed(void) { return _mm_set_epi64x(-1.5, 1u << 31); }
KEEP __m128i set1_epi64x_bit31(void) { return _mm_set1_epi64x(1u << 31); }
KEEP __m128i set1_epi64x_minus1(void) { return _mm_set1_epi64x(-1); }
KEEP __m128i set1_epi64x_uchar(void) { return _mm_set1_epi64x((unsigned char)200); }
KEEP __m128i set1_epi32_schar(void) { return _mm_set1_epi32((signed char)-3); }
KEEP __m128i set1_epi32_char(void) { return _mm_set1_epi32('\377'); }
KEEP __m256i set1_epi32_3e9(void) { return _mm256_set1_epi32(3000000000u); }
KEEP __m256i set1_epi64x_3e9(void) { return _mm256_set1_epi64x(3000000000u); }
KEEP __m256i set1_epi64x_minus3e9(void) { return _mm256_set1_epi64x(-3000000000ll); }

/* ---- floating parameters ---- */
KEEP __m128 set1_ps_float(void) { return _mm_set1_ps(3.7f); }
KEEP __m128 set1_ps_double(void) { return _mm_set1_ps(3.7); }
KEEP __m128 set1_ps_int(void) { return _mm_set1_ps(3); }
KEEP __m128 set_ss_double(void) { return _mm_set_ss(2.5); }
KEEP __m128d set1_pd_float(void) { return _mm_set1_pd(3.7f); }
KEEP __m128d set1_pd_int(void) { return _mm_set1_pd(7); }
KEEP __m128d set_sd_float(void) { return _mm_set_sd(1.25f); }
KEEP __m256 set1_ps_256(void) { return _mm256_set1_ps(3.7); }
KEEP __m256d set1_pd_256(void) { return _mm256_set1_pd(11); }
KEEP __m128 set_ps_mixed(void) { return _mm_set_ps(1.5, 2, 3.25f, 4); }
KEEP __m128d set_pd_mixed(void) { return _mm_set_pd(1, 2.5f); }
KEEP __m256 set_ps_256(void) { return _mm256_set_ps(1, 2.5, 3, 4.5f, 5, 6, 7, 8); }

int main(void)
{
    volatile unsigned seed = 0x11111111u;
    __m128i v = _mm_set_epi32((int)(seed * 4), (int)(seed * 3), (int)(seed * 2), (int)seed);
    __m256i w = _mm256_set_epi32((int)0x80000000u, 7, -6, 5, 0x7fffffff, 3, -200, (int)(seed >> 28));

    ROW("set1_epi32 3.7", __m128i, set1_epi32_double());
    ROW("set1_epi16 3.7f", __m128i, set1_epi16_float());
    ROW("set1_epi8 -3.7", __m128i, set1_epi8_negative());
    ROW("set1_epi64x 3.7", __m128i, set1_epi64x_double());
    ROW("set1_epi64x 3e10", __m128i, set1_epi64x_wide());
    ROW("set_epi32", __m128i, set_epi32_mixed());
    ROW("set_epi16", __m128i, set_epi16_mixed());
    ROW("cvtsi32_si128 9.9", __m128i, cvtsi32_si128_double());
    ROW("cvtsi64_si128", __m128i, cvtsi64_si128_wide());
    ROW("insert_epi32 7.7", __m128i, insert_epi32_double(v));
    ROW("insert_epi16 7.7f", __m128i, insert_epi16_float(v));
    ROW("insert_epi8 7.7", __m128i, insert_epi8_double(v));
    ROW("insert_epi64 7.7", __m128i, insert_epi64_double(v));
    ROW("256 set1_epi8", __m256i, set1_epi8_256());
    ROW("256 set1_epi16", __m256i, set1_epi16_256());
    ROW("256 set1_epi32", __m256i, set1_epi32_256());
    ROW("256 set1_epi64x", __m256i, set1_epi64x_256());
    ROW("256 set_epi32", __m256i, set_epi32_256());
    ROW("256 set_epi64x", __m256i, set_epi64x_256());
    ROW("cvtsi32_ss 5.5", __m128, cvtsi32_ss_double(_mm_castsi128_ps(v)));
    ROW("cvtsi32_sd 6.5f", __m128d, cvtsi32_sd_float(_mm_castsi128_pd(v)));
    ROW("cvtsi64_sd 6.5e10", __m128d, cvtsi64_sd_wide(_mm_castsi128_pd(v)));
    printf("%-18s %08x\n", "crc32_u32", crc32_u32_double());
    printf("%-18s %08x\n", "crc32_u8", crc32_u8_double());
    ROW("slli_epi32 2.9", __m128i, slli_epi32_double(v));
    ROW("srai_epi16 1.5f", __m128i, srai_epi16_float(v));
    ROW("add set1 3.7", __m256i, add_set1_double(w));
    ROW("add set1 -2.9f", __m256i, add_set1_float(w));
    ROW("add set1 128", __m128i, add_set1_128(v));
    ROW("add set1 epi16", __m256i, add_set1_epi16(w));
    ROW("add set1 epi64x", __m256i, add_set1_epi64x(w));
    ROW("add set1 epi8", __m256i, add_set1_epi8(w));

    ROW("epi32 1u<<31", __m128i, set1_epi32_bit31());
    ROW("epi32 33 bits", __m128i, set1_epi32_33bits());
    ROW("epi8 511", __m128i, set1_epi8_511());
    ROW("epi16 -1ll", __m128i, set1_epi16_minus1ll());
    ROW("set_epi64x", __m128i, set_epi64x_mixed());
    ROW("epi64x 1u<<31", __m128i, set1_epi64x_bit31());
    ROW("epi64x -1", __m128i, set1_epi64x_minus1());
    ROW("epi64x uchar", __m128i, set1_epi64x_uchar());
    ROW("epi32 schar", __m128i, set1_epi32_schar());
    ROW("epi32 char", __m128i, set1_epi32_char());
    ROW("256 epi32 3e9", __m256i, set1_epi32_3e9());
    ROW("256 epi64x 3e9", __m256i, set1_epi64x_3e9());
    ROW("256 epi64x -3e9", __m256i, set1_epi64x_minus3e9());

    ROW("set1_ps 3.7f", __m128, set1_ps_float());
    ROW("set1_ps 3.7", __m128, set1_ps_double());
    ROW("set1_ps 3", __m128, set1_ps_int());
    ROW("set_ss 2.5", __m128, set_ss_double());
    ROW("set1_pd 3.7f", __m128d, set1_pd_float());
    ROW("set1_pd 7", __m128d, set1_pd_int());
    ROW("set_sd 1.25f", __m128d, set_sd_float());
    ROW("256 set1_ps", __m256, set1_ps_256());
    ROW("256 set1_pd", __m256d, set1_pd_256());
    ROW("set_ps", __m128, set_ps_mixed());
    ROW("set_pd", __m128d, set_pd_mixed());
    ROW("256 set_ps", __m256, set_ps_256());
    return 0;
}
