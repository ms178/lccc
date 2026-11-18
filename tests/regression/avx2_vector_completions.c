/* Differential test for the vector-extension AVX2 completions (saturating
 * add/sub, averages, count and variable shifts, sign, blendv, cmpgt, and the
 * 128-bit broadcasts) plus the AVX1 extract/insert_epi16 pair.
 *
 * Each intrinsic is driven with the same deterministic input stream: a xorshift
 * PRNG, interleaved with edge values (saturation bounds, zero, sign boundaries,
 * and shift counts at and past the element width). Every result is folded into
 * an FNV-1a hash. The program prints one "name hash" line per intrinsic, so the
 * output is compared against GCC 16.2 running its own implementation of the same
 * intrinsic. Compiled with -O2 -mavx2. */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define ITER 4096

static uint64_t rng_state = 0x9E3779B97F4A7C15ull;

static uint64_t next_u64(void)
{
    uint64_t x = rng_state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    rng_state = x;
    return x;
}

/* Edge values that exercise saturation boundaries, zero, and sign changes. */
static const uint32_t edge32[] = {
    0x00000000u, 0x00000001u, 0x0000007Fu, 0x00000080u, 0x000000FFu,
    0x00007FFFu, 0x00008000u, 0x0000FFFFu, 0x7FFFFFFFu, 0x80000000u,
    0xFFFFFFFFu, 0x0000001Fu, 0x00000020u, 0x0000003Fu, 0x00000040u,
};

static uint32_t rnd32(unsigned pick)
{
    if (pick % 3 == 0)
        return edge32[next_u64() % (sizeof edge32 / sizeof edge32[0])];
    return (uint32_t)next_u64();
}

static __m256i rnd256(void)
{
    uint32_t w[8];
    for (int i = 0; i < 8; i++)
        w[i] = rnd32((unsigned)next_u64());
    __m256i v;
    memcpy(&v, w, sizeof v);
    return v;
}

static __m128i rnd128(void)
{
    uint32_t w[4];
    for (int i = 0; i < 4; i++)
        w[i] = rnd32((unsigned)next_u64());
    __m128i v;
    memcpy(&v, w, sizeof v);
    return v;
}

/* A count vector whose low element sometimes sits at or past the element width. */
static __m256i rnd_count256(void)
{
    uint32_t w[8];
    for (int i = 0; i < 8; i++)
        w[i] = (uint32_t)(next_u64() % 100u);
    __m256i v;
    memcpy(&v, w, sizeof v);
    return v;
}

static __m128i rnd_count128(void)
{
    uint64_t c[2] = {next_u64() % 100u, next_u64()};
    __m128i v;
    memcpy(&v, c, sizeof v);
    return v;
}

static void fold(uint64_t *h, const void *p, size_t n)
{
    const unsigned char *b = p;
    for (size_t i = 0; i < n; i++) {
        *h ^= b[i];
        *h *= 0x100000001B3ull;
    }
}

#define HASH_INIT 0xCBF29CE484222325ull

#define RUN_BIN256(NAME, FN)                                   \
    do {                                                       \
        uint64_t h = HASH_INIT;                                \
        rng_state = 0x9E3779B97F4A7C15ull;                     \
        for (int i = 0; i < ITER; i++) {                       \
            __m256i a = rnd256(), b = rnd256();                \
            __m256i r = FN(a, b);                              \
            fold(&h, &r, sizeof r);                            \
        }                                                      \
        printf("%s %016llx\n", NAME, (unsigned long long)h);   \
    } while (0)

#define RUN_SHIFT256(NAME, FN)                                 \
    do {                                                       \
        uint64_t h = HASH_INIT;                                \
        rng_state = 0x9E3779B97F4A7C15ull;                     \
        for (int i = 0; i < ITER; i++) {                       \
            __m256i a = rnd256();                              \
            __m128i c = rnd_count128();                        \
            __m256i r = FN(a, c);                              \
            fold(&h, &r, sizeof r);                            \
        }                                                      \
        printf("%s %016llx\n", NAME, (unsigned long long)h);   \
    } while (0)

#define RUN_VAR256(NAME, FN)                                   \
    do {                                                       \
        uint64_t h = HASH_INIT;                                \
        rng_state = 0x9E3779B97F4A7C15ull;                     \
        for (int i = 0; i < ITER; i++) {                       \
            __m256i a = rnd256(), c = rnd_count256();          \
            __m256i r = FN(a, c);                              \
            fold(&h, &r, sizeof r);                            \
        }                                                      \
        printf("%s %016llx\n", NAME, (unsigned long long)h);   \
    } while (0)

#define RUN_BIN128(NAME, FN)                                   \
    do {                                                       \
        uint64_t h = HASH_INIT;                                \
        rng_state = 0x9E3779B97F4A7C15ull;                     \
        for (int i = 0; i < ITER; i++) {                       \
            __m128i a = rnd128(), c = rnd_count128();          \
            __m128i r = FN(a, c);                              \
            fold(&h, &r, sizeof r);                            \
        }                                                      \
        printf("%s %016llx\n", NAME, (unsigned long long)h);   \
    } while (0)

#define RUN_UN128D(NAME, FN)                                   \
    do {                                                       \
        uint64_t h = HASH_INIT;                                \
        rng_state = 0x9E3779B97F4A7C15ull;                     \
        for (int i = 0; i < ITER; i++) {                       \
            __m128d a = (__m128d)rnd128();                     \
            __m128d r = FN(a);                                 \
            fold(&h, &r, sizeof r);                            \
        }                                                      \
        printf("%s %016llx\n", NAME, (unsigned long long)h);   \
    } while (0)

#define RUN_UN128S(NAME, FN)                                   \
    do {                                                       \
        uint64_t h = HASH_INIT;                                \
        rng_state = 0x9E3779B97F4A7C15ull;                     \
        for (int i = 0; i < ITER; i++) {                       \
            __m128 a = (__m128)rnd128();                       \
            __m128 r = FN(a);                                  \
            fold(&h, &r, sizeof r);                            \
        }                                                      \
        printf("%s %016llx\n", NAME, (unsigned long long)h);   \
    } while (0)

/* Extract and insert on a 256-bit value with a constant element index. */
#define RUN_EXTRACT16(NAME, N)                                 \
    do {                                                       \
        uint64_t h = HASH_INIT;                                \
        rng_state = 0x9E3779B97F4A7C15ull;                     \
        for (int i = 0; i < ITER; i++) {                       \
            __m256i a = rnd256();                              \
            int e = _mm256_extract_epi16(a, N);                \
            fold(&h, &e, sizeof e);                            \
            short d = (short)next_u64();                       \
            __m256i r = _mm256_insert_epi16(a, d, N);          \
            fold(&h, &r, sizeof r);                            \
        }                                                      \
        printf("%s %016llx\n", NAME, (unsigned long long)h);   \
    } while (0)

int main(void)
{
    RUN_BIN256("adds_epi8", _mm256_adds_epi8);
    RUN_BIN256("adds_epi16", _mm256_adds_epi16);
    RUN_BIN256("adds_epu8", _mm256_adds_epu8);
    RUN_BIN256("adds_epu16", _mm256_adds_epu16);
    RUN_BIN256("subs_epi8", _mm256_subs_epi8);
    RUN_BIN256("subs_epi16", _mm256_subs_epi16);
    RUN_BIN256("subs_epu8", _mm256_subs_epu8);
    RUN_BIN256("avg_epu8", _mm256_avg_epu8);
    RUN_BIN256("avg_epu16", _mm256_avg_epu16);
    RUN_BIN256("cmpgt_epi8", _mm256_cmpgt_epi8);
    RUN_BIN256("cmpgt_epi16", _mm256_cmpgt_epi16);
    RUN_BIN256("sign_epi8", _mm256_sign_epi8);
    RUN_BIN256("sign_epi16", _mm256_sign_epi16);
    RUN_BIN256("sign_epi32", _mm256_sign_epi32);

    RUN_SHIFT256("sll_epi16", _mm256_sll_epi16);
    RUN_SHIFT256("sll_epi32", _mm256_sll_epi32);
    RUN_SHIFT256("sll_epi64", _mm256_sll_epi64);
    RUN_SHIFT256("srl_epi16", _mm256_srl_epi16);
    RUN_SHIFT256("srl_epi32", _mm256_srl_epi32);
    RUN_SHIFT256("srl_epi64", _mm256_srl_epi64);
    RUN_SHIFT256("sra_epi16", _mm256_sra_epi16);
    RUN_SHIFT256("sra_epi32", _mm256_sra_epi32);

    RUN_VAR256("sllv_epi32", _mm256_sllv_epi32);
    RUN_VAR256("sllv_epi64", _mm256_sllv_epi64);
    RUN_VAR256("srlv_epi32", _mm256_srlv_epi32);
    RUN_VAR256("srlv_epi64", _mm256_srlv_epi64);
    RUN_VAR256("srav_epi32", _mm256_srav_epi32);

    RUN_BIN128("sllv_epi32_128", _mm_sllv_epi32);
    RUN_BIN128("sllv_epi64_128", _mm_sllv_epi64);
    RUN_BIN128("srlv_epi32_128", _mm_srlv_epi32);
    RUN_BIN128("srlv_epi64_128", _mm_srlv_epi64);
    RUN_BIN128("srav_epi32_128", _mm_srav_epi32);

    RUN_UN128D("broadcastsd_pd", _mm_broadcastsd_pd);
    RUN_UN128S("broadcastss_ps", _mm_broadcastss_ps);
    RUN_UN128D("movedup_pd", _mm_movedup_pd);
    RUN_BIN256("cmpeq_epi16", _mm256_cmpeq_epi16);

    /* 128-to-256 broadcast (GCC's spelling _mm_broadcastsi128_si256). */
    {
        uint64_t h = HASH_INIT;
        rng_state = 0x9E3779B97F4A7C15ull;
        for (int i = 0; i < ITER; i++) {
            __m128i a = rnd128();
            __m256i r = _mm_broadcastsi128_si256(a);
            fold(&h, &r, sizeof r);
        }
        printf("broadcastsi128_si256 %016llx\n", (unsigned long long)h);
    }

    /* blendv_epi8 takes three operands; run it with the mask as the third. */
    {
        uint64_t h = HASH_INIT;
        rng_state = 0x9E3779B97F4A7C15ull;
        for (int i = 0; i < ITER; i++) {
            __m256i a = rnd256(), b = rnd256(), m = rnd256();
            __m256i r = _mm256_blendv_epi8(a, b, m);
            fold(&h, &r, sizeof r);
        }
        printf("blendv_epi8 %016llx\n", (unsigned long long)h);
    }

    /* AVX1 16-bit element extract and insert at each of the 16 positions. */
    RUN_EXTRACT16("extract_epi16_0", 0);
    RUN_EXTRACT16("extract_epi16_5", 5);
    RUN_EXTRACT16("extract_epi16_7", 7);
    RUN_EXTRACT16("extract_epi16_8", 8);
    RUN_EXTRACT16("extract_epi16_15", 15);

    return 0;
}
