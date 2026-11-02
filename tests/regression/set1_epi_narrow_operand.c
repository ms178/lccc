/* Regression test: the GPR-source vpbroadcast lowering must materialise its
 * operand through a 64-bit register.
 *
 * `_mm256_set1_epi{8,16,32,64}` with a RUNTIME operand lowers to
 * "load the value into a GPR, movd/movq it into xmm0, vpbroadcast". The load
 * step went through operand_to_reg(), which always emits a 64-bit
 * `movq %src, %dst` -- but it was handed the 32-bit name "eax" for the 16- and
 * 32-bit element widths. That produced
 *
 *     movq %r8, %eax        <- not encodable: 64-bit source, 32-bit destination
 *
 * which the integrated assembler correctly rejected with
 * "operand size mismatch for `movq`". zlib-ng could not be built at all:
 * arch/x86/slide_hash_avx2.c does `_mm256_set1_epi16((short)wsize)` where
 * wsize is narrowed from a wider struct field, and the file failed to compile.
 *
 * The fix loads through %rax and lets the following movd/movq express the
 * narrowing, so the emitted code is now one instruction SHORTER as well
 * (`movd %r8d, %xmm0` directly).
 *
 * Every element width is covered, and each broadcast operand is narrowed from
 * a wider value so the lowering really does see a truncating source rather
 * than a same-width copy. Results are checked elementwise at runtime.
 */

#include <immintrin.h>
#include <stdint.h>
#include <string.h>

__attribute__((noinline))
static __m256i b8(uint32_t w)  { return _mm256_set1_epi8((char)(uint8_t)w); }
__attribute__((noinline))
static __m256i b16(uint32_t w) { return _mm256_set1_epi16((short)(uint16_t)w); }
__attribute__((noinline))
static __m256i b32(uint64_t w) { return _mm256_set1_epi32((int)(uint32_t)w); }
__attribute__((noinline))
static __m256i b64(uint64_t w) { return _mm256_set1_epi64x((long long)w); }

int main(void)
{
    /* 0x1234 truncated to 8 bits is 0x34; to 16 bits it stays 0x1234. */
    {
        unsigned char out[32];
        _mm256_storeu_si256((__m256i *)out, b8(0x1234u));
        for (int i = 0; i < 32; i++) if (out[i] != 0x34) return 1;
    }
    {
        uint16_t out[16];
        _mm256_storeu_si256((__m256i *)out, b16(0xABCD1234u));
        for (int i = 0; i < 16; i++) if (out[i] != 0x1234) return 2;
    }
    {
        uint32_t out[8];
        _mm256_storeu_si256((__m256i *)out, b32(0xDEADBEEFCAFEBABEull));
        for (int i = 0; i < 8; i++) if (out[i] != 0xCAFEBABEu) return 3;
    }
    {
        uint64_t out[4];
        _mm256_storeu_si256((__m256i *)out, b64(0x0123456789ABCDEFull));
        for (int i = 0; i < 4; i++) if (out[i] != 0x0123456789ABCDEFull) return 4;
    }

    /* The zlib-ng slide_hash shape verbatim: saturating 16-bit subtract of a
     * broadcast window size, which is what the miscompiled file computes. */
    {
        uint16_t table[16], expect[16];
        for (int i = 0; i < 16; i++) table[i] = (uint16_t)(i * 4096);
        uint32_t w_size = 32768;
        __m256i wsize = b16(w_size);
        __m256i v = _mm256_loadu_si256((const __m256i *)table);
        __m256i r = _mm256_subs_epu16(v, wsize);
        _mm256_storeu_si256((__m256i *)table, r);
        for (int i = 0; i < 16; i++) {
            int t = i * 4096 - 32768;
            expect[i] = (uint16_t)(t < 0 ? 0 : t);
        }
        if (memcmp(table, expect, sizeof table) != 0) return 5;
    }

    return 0;
}
