/* lccc's MMX/SSE2/AVX2 set intrinsics are C bodies, and built a 64-bit lane
 * word as `(long long)(unsigned T)x << 32` (also << 48, << 56): a signed
 * shift that overflows whenever the top lane has its sign bit set, which is
 * undefined behaviour. Under -fsanitize=undefined -fsanitize-trap=undefined
 * every such call trapped (_mm_set1_pi8(-1), _mm_setr_epi16 with a negative
 * fourth lane, ...), where GCC's builtin-based headers run clean. Each form is
 * called with operands whose top lanes are negative, positive and the type
 * minimum, and the lane bytes are hashed against GCC. */
#include <stdio.h>
#include <string.h>
#include <emmintrin.h>
#ifdef __AVX2__
#include <immintrin.h>
#endif

static unsigned long long h = 1469598103934665603ULL;
static void mix(const void *p, unsigned n)
{
    const unsigned char *b = p;
    for (unsigned i = 0; i < n; ++i)
        h = (h ^ b[i]) * 1099511628211ULL;
}
#define MIX(v) do { __typeof__(v) t_ = (v); mix(&t_, sizeof t_); } while (0)

int main(void)
{
    static const int vals[] = {-1, 1, -2147483647 - 1, 0x7fffffff, -129, 0x12345678};
    for (volatile unsigned k = 0; k < sizeof vals / sizeof vals[0]; ++k) {
        int x = vals[k], y = vals[(k + 1) % 6];
        short s = (short)x, t = (short)y;
        char c = (char)x, d = (char)y;
        MIX(_mm_set_pi32(x, y));
        MIX(_mm_setr_pi32(x, y));
        MIX(_mm_set1_pi32(x));
        MIX(_mm_set_pi16(s, t, s, t));
        MIX(_mm_setr_pi16(s, t, t, s));
        MIX(_mm_set1_pi16(s));
        MIX(_mm_set_pi8(c, d, c, d, d, c, d, c));
        MIX(_mm_set1_pi8(c));
        MIX(_mm_set1_epi32(x));
        MIX(_mm_set1_epi16(s));
        MIX(_mm_setr_epi16(s, t, s, t, t, s, t, s));
        MIX(_mm_set_epi16(s, t, s, t, t, s, t, s));
        MIX(_mm_setr_epi32(x, y, x, y));
        MIX(_mm_set_epi32(x, y, y, x));
        MIX(_mm_set_epi8(c, d, c, d, d, c, d, c, c, c, d, d, c, d, c, d));
        MIX(_mm_setr_epi8(c, d, c, d, d, c, d, c, c, c, d, d, c, d, c, d));
        __m128i a = _mm_set_epi32(x, y, ~x, ~y), b = _mm_set_epi32(~y, x, y, ~x);
        MIX(_mm_unpacklo_epi32(a, b));
        MIX(_mm_unpackhi_epi32(a, b));
#ifdef __AVX2__
        MIX(_mm256_set1_epi32(x));
#endif
    }
    _mm_empty();
    printf("set lanes %016llx\n", h);
    return 0;
}
