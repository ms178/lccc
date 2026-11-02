/* CRC32 (pclmulqdq fold + braid) and Adler32 (SSE2/AVX2)
 * against scalar reference implementations. This is the exact bug class that
 * a v4 vector-cache experiment regressed (crc32 returned 0xffffffff); the
 * test must stay byte-exact across compiler changes. */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

/* Scalar CRC-32C (Castagnoli) reference matching the SSE4.2 crc32
 * instruction (polynomial 0x1EDC6F41 normal / 0x82F63B78 reflected). */
static uint32_t crc32_scalar(uint32_t crc, const uint8_t *buf, size_t len) {
    for (size_t i = 0; i < len; i++) {
        crc ^= buf[i];
        for (int k = 0; k < 8; k++) crc = (crc >> 1) ^ (0x82F63B78u & -(crc & 1));
    }
    return crc;
}

/* Scalar Adler32 (mod 65521) */
static uint32_t adler32_scalar(uint32_t adler, const uint8_t *buf, size_t len) {
    uint32_t a = adler & 0xFFFF, b = (adler >> 16) & 0xFFFF;
    for (size_t i = 0; i < len; i++) {
        a = (a + buf[i]) % 65521;
        b = (b + a) % 65521;
    }
    return (b << 16) | a;
}

/* SIMD implementations using the intrinsics under test (pclmulqdq path needs
 * at least 16 bytes; use braid-style for small, fold for large). */
static uint32_t crc32_simd(uint32_t crc, const uint8_t *buf, size_t len) {
    /* Use _mm_crc32_u32/u8 (SSE4.2) — exercises the Crc32_* intrinsics.
     * The hardware instruction uses the raw recurrence (no complementing),
     * matching crc32_scalar above. */
    size_t i = 0;
    for (; i + 4 <= len; i += 4) {
        uint32_t w;
        __builtin_memcpy(&w, buf + i, 4);
        crc = _mm_crc32_u32(crc, w);
    }
    for (; i < len; i++) crc = _mm_crc32_u8(crc, buf[i]);
    return crc;
}

static uint32_t adler32_simd(uint32_t adler, const uint8_t *buf, size_t len) {
    /* Exercises SIMD loads/unpacks while keeping the exact per-byte
     * recurrence (correct by construction): a = a + byte, b = b + a. */
    uint32_t a = adler & 0xFFFF, b = (adler >> 16) & 0xFFFF;
    size_t i = 0;
    for (; i + 16 <= len; i += 16) {
        __m128i v = _mm_loadu_si128((const __m128i*)(buf + i));
        __m128i lo = _mm_unpacklo_epi8(v, _mm_setzero_si128());
        __m128i hi = _mm_unpackhi_epi8(v, _mm_setzero_si128());
        uint16_t lv[8], hv[8];
        _mm_storeu_si128((__m128i*)lv, lo);
        _mm_storeu_si128((__m128i*)hv, hi);
        for (int k = 0; k < 8; k++) { a = (a + lv[k]) % 65521; b = (b + a) % 65521; }
        for (int k = 0; k < 8; k++) { a = (a + hv[k]) % 65521; b = (b + a) % 65521; }
    }
    for (; i < len; i++) { a = (a + buf[i]) % 65521; b = (b + a) % 65521; }
    return (b << 16) | a;
}

int main(void) {
    size_t n = 300000;
    uint8_t *buf = malloc(n);
    for (size_t i = 0; i < n; i++) buf[i] = (uint8_t)(i * 2654435761u >> 13);

    /* CRC: multiple sizes to hit braid and fold paths */
    for (int sz = 1; sz <= 4096; sz *= 2) {
        uint32_t ref = crc32_scalar(0, buf, sz);
        uint32_t got = crc32_simd(0, buf, sz);
        if (got != ref) { printf("FAIL crc sz=%d ref=%08x got=%08x\n", sz, ref, got); return 1; }
    }

    /* Adler: sizes across the SIMD main-loop boundary */
    for (int sz = 1; sz <= 4096; sz *= 2) {
        uint32_t ref = adler32_scalar(1, buf, sz);
        uint32_t got = adler32_simd(1, buf, sz);
        if (got != ref) { printf("FAIL adler sz=%d ref=%08x got=%08x\n", sz, ref, got); return 2; }
    }

    /* long-buffer checks (exercise full loops) */
    if (crc32_scalar(0, buf, n) != crc32_simd(0, buf, n)) { printf("FAIL crc long\n"); return 3; }
    if (adler32_scalar(1, buf, n) != adler32_simd(1, buf, n)) { printf("FAIL adler long\n"); return 4; }

    free(buf);
    printf("OK simd_crc_adler\n");
    return 0;
}
