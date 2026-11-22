/* Regression fixture (register-result forwarding). Native AVX2 chains over the register-form 256-bit producers that the
   Result-forwarding now routes through register homes. Runs on any AVX2 host.
   Output: one hex line per case; compare lccc build against GCC 16.2. */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

static uint64_t seed = 0x9E3779B97F4A7C15ull;
static uint64_t rnd(void) {
    seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
    return seed;
}

__attribute__((noinline))
static void chain_perm(const __m256i *a, const __m256i *b, __m256i *out) {
    __m256i x = _mm256_loadu_si256(a);
    __m256i y = _mm256_loadu_si256(b);
    __m256i p = _mm256_permute2x128_si256(x, y, 0x21);
    __m256i q = _mm256_permute4x64_epi64(p, 0x39);
    __m256i r = _mm256_shuffle_epi32(q, 0x4e);
    __m256i s = _mm256_blend_epi32(r, x, 0x0f);
    __m256i t = _mm256_add_epi64(s, y);
    _mm256_storeu_si256(out, t);
}

__attribute__((noinline))
static void chain_shuf(const __m256i *a, const __m256i *b, const __m256i *m, __m256i *out) {
    __m256i x = _mm256_loadu_si256(a);
    __m256i y = _mm256_loadu_si256(b);
    __m256i mk = _mm256_loadu_si256(m);
    __m256i p = _mm256_shuffle_epi8(x, mk);
    __m256i q = _mm256_permute2x128_si256(p, y, 0x13);
    __m256i r = _mm256_shuffle_epi8(q, mk);
    __m256i s = _mm256_xor_si256(r, p);
    _mm256_storeu_si256(out, s);
}

__attribute__((noinline))
static void chain_extract(const __m256i *a, const __m256i *b, __m128i *out) {
    __m256i x = _mm256_loadu_si256(a);
    __m256i y = _mm256_loadu_si256(b);
    __m256i p = _mm256_permute4x64_epi64(x, 0x1b);
    __m128i lo = _mm256_extracti128_si256(p, 0);
    __m128i hi = _mm256_extracti128_si256(y, 1);
    __m128i q = _mm_add_epi64(lo, hi);
    __m256i w = _mm256_permute2x128_si256(x, y, 0x20);
    __m128i z = _mm256_extracti128_si256(w, 1);
    _mm_storeu_si128(out, _mm_xor_si128(q, z));
}

__attribute__((noinline))
static void chain_blend(const __m256i *a, const __m256i *b, __m256i *out) {
    __m256 x = _mm256_castsi256_ps(_mm256_loadu_si256(a));
    __m256 y = _mm256_castsi256_ps(_mm256_loadu_si256(b));
    __m256 p = _mm256_blend_ps(x, y, 0xa5);
    __m256 q = _mm256_blend_ps(p, x, 0x3c);
    _mm256_storeu_si256(out, _mm256_castps_si256(q));
}

static void dump(const char *tag, const void *p, size_t n) {
    const unsigned char *b = p;
    printf("%s ", tag);
    for (size_t i = 0; i < n; i++) printf("%02x", b[i]);
    printf("\n");
}

int main(void) {
    for (int iter = 0; iter < 64; iter++) {
        __m256i a, b, m, o;
        uint64_t va[4], vb[4], vm[4];
        for (int i = 0; i < 4; i++) { va[i] = rnd(); vb[i] = rnd(); }
        /* shuffle mask: bytes in 0..15 per lane, high bit clear */
        for (int i = 0; i < 4; i++) {
            uint64_t w = 0;
            for (int k = 0; k < 8; k++) w |= (uint64_t)((rnd() & 0x0f) | (k == 0 ? 0 : 0)) << (8 * k);
            vm[i] = w;
        }
        memcpy(&a, va, 32); memcpy(&b, vb, 32); memcpy(&m, vm, 32);
        chain_perm(&a, &b, &o); dump("perm", &o, 32);
        chain_shuf(&a, &b, &m, &o); dump("shuf", &o, 32);
        __m128i e; chain_extract(&a, &b, &e); dump("extr", &e, 16);
        chain_blend(&a, &b, &o); dump("blnd", &o, 32);
    }
    return 0;
}
