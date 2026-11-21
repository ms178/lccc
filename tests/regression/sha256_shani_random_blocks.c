/* SHA-256 compression through the SHA-NI intrinsics, differentially checked
 * against a portable C reference on random blocks and random chaining states.
 *
 * The block function is the Intel SHA-NI schedule (`sha256msg1` / `alignr` /
 * `paddd` / `sha256msg2` for the message, `sha256rnds2` twice per four rounds),
 * with every message word and round key held in an __m128i. Any register
 * allocation, copy or spill error in the SHA-256 chain therefore changes a
 * state word, and the portable reference (plain FIPS 180-4 arithmetic, no
 * intrinsics) catches it.
 *
 * Exit status is 0 only when every random block matches the reference and the
 * FIPS 180-4 known-answer vector for "abc" matches. The program also prints a
 * FNV-1a hash of all reference outputs, so the GCC 16.2 oracle binary must
 * print the same bytes, not only exit 0.
 *
 * Build flags: -O2 -msha -mssse3 -msse4.1 (see the .flags sidecar). The host
 * must run SHA-NI.
 */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

static const uint32_t K256[64] = {
    0x428a2f98U, 0x71374491U, 0xb5c0fbcfU, 0xe9b5dba5U, 0x3956c25bU, 0x59f111f1U, 0x923f82a4U, 0xab1c5ed5U,
    0xd807aa98U, 0x12835b01U, 0x243185beU, 0x550c7dc3U, 0x72be5d74U, 0x80deb1feU, 0x9bdc06a7U, 0xc19bf174U,
    0xe49b69c1U, 0xefbe4786U, 0x0fc19dc6U, 0x240ca1ccU, 0x2de92c6fU, 0x4a7484aaU, 0x5cb0a9dcU, 0x76f988daU,
    0x983e5152U, 0xa831c66dU, 0xb00327c8U, 0xbf597fc7U, 0xc6e00bf3U, 0xd5a79147U, 0x06ca6351U, 0x14292967U,
    0x27b70a85U, 0x2e1b2138U, 0x4d2c6dfcU, 0x53380d13U, 0x650a7354U, 0x766a0abbU, 0x81c2c92eU, 0x92722c85U,
    0xa2bfe8a1U, 0xa81a664bU, 0xc24b8b70U, 0xc76c51a3U, 0xd192e819U, 0xd6990624U, 0xf40e3585U, 0x106aa070U,
    0x19a4c116U, 0x1e376c08U, 0x2748774cU, 0x34b0bcb5U, 0x391c0cb3U, 0x4ed8aa4aU, 0x5b9cca4fU, 0x682e6ff3U,
    0x748f82eeU, 0x78a5636fU, 0x84c87814U, 0x8cc70208U, 0x90befffaU, 0xa4506cebU, 0xbef9a3f7U, 0xc67178f2U,
};

static const uint32_t IV256[8] = {
    0x6a09e667U, 0xbb67ae85U, 0x3c6ef372U, 0xa54ff53aU,
    0x510e527fU, 0x9b05688cU, 0x1f83d9abU, 0x5be0cd19U,
};

/* Portable reference: FIPS 180-4 section 6.2.2, no intrinsics. */
static inline uint32_t rotr32(uint32_t x, int n) { return (x >> n) | (x << (32 - n)); }

static void ref_compress(uint32_t h[8], const uint8_t blk[64]) {
    uint32_t w[64];
    for (int t = 0; t < 16; t++)
        w[t] = ((uint32_t)blk[4 * t] << 24) | ((uint32_t)blk[4 * t + 1] << 16) |
               ((uint32_t)blk[4 * t + 2] << 8) | (uint32_t)blk[4 * t + 3];
    for (int t = 16; t < 64; t++) {
        uint32_t s0 = rotr32(w[t - 15], 7) ^ rotr32(w[t - 15], 18) ^ (w[t - 15] >> 3);
        uint32_t s1 = rotr32(w[t - 2], 17) ^ rotr32(w[t - 2], 19) ^ (w[t - 2] >> 10);
        w[t] = w[t - 16] + s0 + w[t - 7] + s1;
    }
    uint32_t a = h[0], b = h[1], c = h[2], d = h[3], e = h[4], f = h[5], g = h[6], hh = h[7];
    for (int t = 0; t < 64; t++) {
        uint32_t S1 = rotr32(e, 6) ^ rotr32(e, 11) ^ rotr32(e, 25);
        uint32_t ch = (e & f) ^ (~e & g);
        uint32_t t1 = hh + S1 + ch + K256[t] + w[t];
        uint32_t S0 = rotr32(a, 2) ^ rotr32(a, 13) ^ rotr32(a, 22);
        uint32_t maj = (a & b) ^ (a & c) ^ (b & c);
        uint32_t t2 = S0 + maj;
        hh = g; g = f; f = e; e = d + t1; d = c; c = b; b = a; a = t1 + t2;
    }
    h[0] += a; h[1] += b; h[2] += c; h[3] += d;
    h[4] += e; h[5] += f; h[6] += g; h[7] += hh;
}

/* SHA-NI compression. `h` holds A..H in dword order. */
static void shani_compress(uint32_t h[8], const uint8_t blk[64]) {
    const __m128i bswap = _mm_set_epi8(12, 13, 14, 15, 8, 9, 10, 11, 4, 5, 6, 7, 0, 1, 2, 3);
    __m128i tmp = _mm_loadu_si128((const __m128i *)&h[0]);
    __m128i st1 = _mm_loadu_si128((const __m128i *)&h[4]);
    tmp = _mm_shuffle_epi32(tmp, 0xB1);          /* CDAB */
    st1 = _mm_shuffle_epi32(st1, 0x1B);          /* EFGH */
    __m128i st0 = _mm_alignr_epi8(tmp, st1, 8);  /* ABEF */
    st1 = _mm_blend_epi16(st1, tmp, 0xF0);       /* CDGH */
    const __m128i abef_save = st0, cdgh_save = st1;

    __m128i w[16];
    for (int g = 0; g < 4; g++)
        w[g] = _mm_shuffle_epi8(_mm_loadu_si128((const __m128i *)(blk + 16 * g)), bswap);
    for (int g = 4; g < 16; g++) {
        __m128i t = _mm_alignr_epi8(w[g - 1], w[g - 2], 4);
        __m128i m = _mm_sha256msg1_epu32(w[g - 4], w[g - 3]);
        m = _mm_add_epi32(m, t);
        w[g] = _mm_sha256msg2_epu32(m, w[g - 1]);
    }
    for (int g = 0; g < 16; g++) {
        __m128i msg = _mm_add_epi32(w[g], _mm_loadu_si128((const __m128i *)&K256[4 * g]));
        st1 = _mm_sha256rnds2_epu32(st1, st0, msg);
        msg = _mm_shuffle_epi32(msg, 0x0E);
        st0 = _mm_sha256rnds2_epu32(st0, st1, msg);
    }
    st0 = _mm_add_epi32(st0, abef_save);
    st1 = _mm_add_epi32(st1, cdgh_save);

    tmp = _mm_shuffle_epi32(st0, 0x1B);          /* FEBA */
    st1 = _mm_shuffle_epi32(st1, 0xB1);          /* DCHG */
    st0 = _mm_blend_epi16(tmp, st1, 0xF0);       /* DCBA */
    st1 = _mm_alignr_epi8(st1, tmp, 8);          /* HGFE */
    _mm_storeu_si128((__m128i *)&h[0], st0);
    _mm_storeu_si128((__m128i *)&h[4], st1);
}

static uint64_t rng_state = 0x9e3779b97f4a7c15ULL;
static uint32_t rng32(void) {
    /* SplitMix64: deterministic across compilers, no libc rand(). */
    uint64_t z = (rng_state += 0x9e3779b97f4a7c15ULL);
    z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9ULL;
    z = (z ^ (z >> 27)) * 0x94d049bb133111ebULL;
    return (uint32_t)((z ^ (z >> 31)) >> 16);
}

int main(void) {
    /* FIPS 180-4 known answer: SHA-256("abc"), one padded block. */
    uint8_t kat[64];
    memset(kat, 0, sizeof kat);
    kat[0] = 'a';
    kat[1] = 'b';
    kat[2] = 'c';
    kat[3] = 0x80;
    kat[63] = 24;
    uint32_t h[8];
    memcpy(h, IV256, sizeof h);
    shani_compress(h, kat);
    static const uint32_t abc[8] = {
        0xba7816bfU, 0x8f01cfeaU, 0x414140deU, 0x5dae2223U,
        0xb00361a3U, 0x96177a9cU, 0xb410ff61U, 0xf20015adU,
    };
    if (memcmp(h, abc, sizeof abc) != 0) {
        printf("KAT mismatch\n");
        return 1;
    }

    enum { BLOCKS = 2000 };
    uint64_t fnv = 0xcbf29ce484222325ULL;
    unsigned mismatches = 0;
    for (int n = 0; n < BLOCKS; n++) {
        uint8_t blk[64];
        uint32_t start[8], hs[8], hr[8];
        for (int i = 0; i < 64; i++) blk[i] = (uint8_t)rng32();
        for (int i = 0; i < 8; i++) start[i] = rng32();
        /* Every 8th state is the IV, so the all-zero-diff path is covered too. */
        if ((n & 7) == 0) memcpy(start, IV256, sizeof start);
        memcpy(hs, start, sizeof hs);
        memcpy(hr, start, sizeof hr);
        shani_compress(hs, blk);
        ref_compress(hr, blk);
        if (memcmp(hs, hr, sizeof hs) != 0) {
            if (mismatches < 4)
                printf("block %d mismatch: got %08x reference %08x\n", n, hs[0], hr[0]);
            mismatches++;
        }
        for (int i = 0; i < 8; i++) {
            for (int b = 0; b < 4; b++) {
                fnv ^= (uint8_t)(hr[i] >> (8 * b));
                fnv *= 0x100000001b3ULL;
            }
        }
    }
    printf("blocks=%d mismatches=%u fnv=%016llx\n", BLOCKS, mismatches,
           (unsigned long long)fnv);
    return mismatches ? 1 : 0;
}
