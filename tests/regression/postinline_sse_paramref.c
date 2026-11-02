/*
 * Regression: post-structural inlining of a helper whose ParamRef values feed
 * SSE intrinsic pointer operands must materialize caller-defined values before
 * removing the cloned ParamRef instructions.  This mirrors zlib-ng's
 * loadchunk/storechunk shape without requiring an external package build.
 */
#include <emmintrin.h>
#include <stdint.h>
#include <stdio.h>

static inline void loadchunk(const uint8_t *src, __m128i *chunk) {
    *chunk = _mm_loadu_si128((const __m128i *)src);
}

static inline void storechunk(uint8_t *dst, const __m128i *chunk) {
    _mm_storeu_si128((__m128i *)dst, *chunk);
}

static void chunkcopy(uint8_t *dst, const uint8_t *src, unsigned n) {
    while (n >= 16U) {
        __m128i chunk;
        loadchunk(src, &chunk);
        storechunk(dst, &chunk);
        src += 16;
        dst += 16;
        n -= 16U;
    }
    while (n) {
        *dst++ = *src++;
        n--;
    }
}

int main(void) {
    uint8_t src[97];
    uint8_t dst[97];
    unsigned sum = 0;
    for (unsigned i = 0; i < 97; ++i) {
        src[i] = (uint8_t)(i * 29U + 7U);
        dst[i] = 0;
    }
    chunkcopy(dst, src, 97);
    for (unsigned i = 0; i < 97; ++i) {
        if (dst[i] != src[i])
            return 2;
        sum += dst[i];
    }
    printf("%u\n", sum);
    return 0;
}
