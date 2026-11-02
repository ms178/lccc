/* GNU raw counts are bits; public byte shifts must not wrap or cross lanes. */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>
static void dump(const void *p, unsigned n) {
    const unsigned char *bytes = p;
    for (unsigned i = 0; i < n; ++i) printf("%02x", bytes[i]);
    puts("");
}
#define CHECK(N) do { \
    __m128i l = _mm_slli_si128(a, N); dump(&l, sizeof l); \
    __m128i r = _mm_srli_si128(a, N); dump(&r, sizeof r); \
    __m128i c = _mm_alignr_epi8(a, b, N); dump(&c, sizeof c); \
    __m256i wl = _mm256_slli_si256(w, N); dump(&wl, sizeof wl); \
    __m256i wr = _mm256_srli_si256(w, N); dump(&wr, sizeof wr); \
} while (0)
int main(void) {
    unsigned char bytes[32];
    for (unsigned i = 0; i < sizeof bytes; ++i) bytes[i] = (unsigned char)(i + 1);
    __m128i a, b;
    __m256i w;
    memcpy(&a, bytes, sizeof a);
    memcpy(&b, bytes + 16, sizeof b);
    memcpy(&w, bytes, sizeof w);
    CHECK(0); CHECK(1); CHECK(15); CHECK(16);
    CHECK(17); CHECK(31); CHECK(32); CHECK(255);
    return 0;
}
