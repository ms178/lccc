/* AVX2 mask/blend intrinsics must lower to their single instruction.
 *
 * Two functions in the bundled <avx2intrin.h> were scalar loops over the whole
 * vector:
 *
 *   `_mm256_movemask_epi8` spilled the vector to the stack and tested 32 bytes
 *   one at a time (`if (__pa[__i] & 0x80) __mask |= 1 << __i;`), i.e. 32
 *   branches for one `vpmovmskb`.  It is the inner operation of every
 *   byte-classification loop -- JSON/XML scanning, UTF-8 validation, memchr
 *   overlays -- so the loop, not the caller, was the hot code.
 *
 *   `_mm256_blend_epi32` copied both operands to the stack and wrote the
 *   result lane by lane, for one immediate blend instruction.
 *
 * Both are now the instruction.  The fixture prints the mask for patterns that
 * exercise every byte and the blend result for every immediate that matters,
 * so a "fixed" shape that computes the wrong bits fails the differential
 * rather than passing on instruction counts.
 */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

static unsigned char src[32] __attribute__((aligned(32)));
static unsigned char src2[32] __attribute__((aligned(32)));
static float fv[8] __attribute__((aligned(32)));
static float fv2[8] __attribute__((aligned(32)));

/* Only the intrinsic: the gate censuses these bodies. */
static __attribute__((noinline)) int mask_of(const void *p)
{
    return _mm256_movemask_epi8(_mm256_loadu_si256((const __m256i *)p));
}

static __attribute__((noinline)) int blend_a5(const void *a, const void *b)
{
    __m256i va = _mm256_loadu_si256((const __m256i *)a);
    __m256i vb = _mm256_loadu_si256((const __m256i *)b);
    int out[8];
    _mm256_storeu_si256((__m256i_u *)out, _mm256_blend_epi32(va, vb, 0xa5));
    return out[0] ^ (out[1] << 1) ^ (out[2] << 2) ^ (out[3] << 3) ^ (out[4] << 4)
           ^ (out[5] << 5) ^ (out[6] << 6) ^ (out[7] << 7);
}

static __attribute__((noinline)) int blend_00(const void *a, const void *b)
{
    __m256i va = _mm256_loadu_si256((const __m256i *)a);
    __m256i vb = _mm256_loadu_si256((const __m256i *)b);
    int out[8];
    _mm256_storeu_si256((__m256i_u *)out, _mm256_blend_epi32(va, vb, 0x00));
    return out[0] ^ (out[1] << 1) ^ (out[7] << 7);
}

static __attribute__((noinline)) int blend_ff(const void *a, const void *b)
{
    __m256i va = _mm256_loadu_si256((const __m256i *)a);
    __m256i vb = _mm256_loadu_si256((const __m256i *)b);
    int out[8];
    _mm256_storeu_si256((__m256i_u *)out, _mm256_blend_epi32(va, vb, 0xff));
    return out[0] ^ (out[1] << 1) ^ (out[7] << 7);
}

static __attribute__((noinline)) int blend_55(const void *a, const void *b)
{
    __m256i va = _mm256_loadu_si256((const __m256i *)a);
    __m256i vb = _mm256_loadu_si256((const __m256i *)b);
    int out[8];
    _mm256_storeu_si256((__m256i_u *)out, _mm256_blend_epi32(va, vb, 0x55));
    return out[0] ^ (out[1] << 1) ^ (out[7] << 7);
}

/* `_mm256_blend_ps` on the same bits: the two must agree where the immediate
 * selects the same lanes, which is what makes the vblendps substitution safe. */
static __attribute__((noinline)) float blend_ps(void)
{
    __m256 a = _mm256_loadu_ps(fv);
    __m256 b = _mm256_loadu_ps(fv2);
    float out[8];
    _mm256_storeu_ps(out, _mm256_blend_ps(a, b, 0x5a));
    float s = 0.0f;
    for (int i = 0; i < 8; i++)
        s += out[i];
    return s;
}

int main(void)
{
    for (int round = 0; round < 8; round++) {
        for (int i = 0; i < 32; i++) {
            src[i] = (unsigned char)((i * 37 + round * 11) & 0xff);
            src2[i] = (unsigned char)(((i ^ 0x5a) * 13 + round) & 0xff);
            if ((i + round) % 5 == 0)
                src[i] |= 0x80;
            if ((i + round) % 7 == 0)
                src2[i] |= 0x80;
        }
        printf("mask %08x mask2 %08x\n", (unsigned)mask_of(src), (unsigned)mask_of(src2));
        printf("blend %d %d %d %d\n", blend_a5(src, src2), blend_00(src, src2),
               blend_ff(src, src2), blend_55(src, src2));
    }

    for (int i = 0; i < 8; i++) {
        fv[i] = (float)i - 3.5f;
        fv2[i] = (float)i * 2.0f + 1.0f;
    }
    float s = blend_ps();
    unsigned bits;
    memcpy(&bits, &s, sizeof(bits));
    printf("blend_ps_sum %08x\n", bits);
    return 0;
}
