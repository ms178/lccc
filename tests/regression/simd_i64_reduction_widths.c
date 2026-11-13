/*
 * 64-bit reduction width and tail discipline.
 *
 * The I64 sum reduction has two lowering arms: a 2-lane 128-bit arm for
 * targets without AVX2 and a 4-lane 256-bit arm for targets with it. Both
 * compute a modular sum, so both must agree with the scalar reference for
 * EVERY trip count -- including the ones that leave 1..3 (128-bit) or 1..7
 * (256-bit) elements for the scalar remainder, and the ones shorter than one
 * vector. A width change that drops a tail element, double-counts the vector
 * body or mis-scales the byte IV is invisible in an instruction count and
 * shows up here.
 *
 * The trip count is always a runtime value: a literal would let the
 * vectorizer take the constant-divisibility fast path and never exercise the
 * remainder at all.
 *
 * Side-effect free; prints one checksum line per case, so the differential
 * oracle (GCC) must reproduce every intermediate value.
 */
#include <stdint.h>
#include <stdio.h>

#define MAXN 41
#define ROUNDS 3

static uint64_t u64data[MAXN + 1];
static int64_t i64data[MAXN + 1];
static uint32_t u32data[MAXN + 1];

static uint64_t sum_u64(const uint64_t *a, size_t n) {
    uint64_t s = 0;
    for (size_t i = 0; i < n; i++) s += a[i];
    return s;
}

static uint64_t sum_u64_byte_iv(const uint64_t *a, size_t n) {
    /* Pointer-marching spelling: a different IV shape for the same reduction,
       which takes the byte-IV path inside the transform. */
    uint64_t s = 0;
    const uint64_t *p = a;
    for (size_t i = 0; i < n; i++) s += *p++;
    return s;
}

static int64_t sum_i64(const int64_t *a, size_t n) {
    int64_t s = 0;
    for (size_t i = 0; i < n; i++) s += a[i];
    return s;
}

static uint64_t sum_u32_into_u64(const uint32_t *a, size_t n) {
    uint64_t s = 0;
    for (size_t i = 0; i < n; i++) s += a[i];
    return s;
}

static uint64_t sum_u64_offset(const uint64_t *a, size_t n) {
    /* Non-zero element start: the transform's element-0 / byte-IV proof must
       either be satisfied exactly or refuse the loop. */
    uint64_t s = 0;
    for (size_t i = 1; i <= n; i++) s += a[i];
    return s;
}

static uint64_t mix(uint64_t h, uint64_t v) {
    h ^= v + 0x9e3779b97f4a7c15ULL + (h << 6) + (h >> 2);
    return h;
}

int main(void) {
    uint64_t seed = 0x0123456789abcdefULL;
    for (size_t i = 0; i <= MAXN; i++) {
        seed = seed * 6364136223846793005ULL + 1442695040888963407ULL;
        u64data[i] = seed;
        i64data[i] = (int64_t)seed >> (unsigned)(i & 7);
        u32data[i] = (uint32_t)(seed >> 32);
    }

    uint64_t h = 0;
    for (int r = 0; r < ROUNDS; r++) {
        for (size_t n = 0; n <= MAXN; n++) {
            h = mix(h, sum_u64(u64data, n));
            h = mix(h, sum_u64_byte_iv(u64data, n));
            h = mix(h, (uint64_t)sum_i64(i64data, n));
            h = mix(h, sum_u32_into_u64(u32data, n));
            h = mix(h, sum_u64_offset(u64data, n > 0 ? n - 1 : 0));
        }
    }

    /* Print the per-trip-count sums for the widest case too: a single final
       checksum could hide two compensating errors, whereas the raw sequence
       pins each n independently. */
    for (size_t n = 0; n <= MAXN; n++) {
        printf("%zu %llu %llu %lld %llu\n", n,
               (unsigned long long)sum_u64(u64data, n),
               (unsigned long long)sum_u64_byte_iv(u64data, n),
               (long long)sum_i64(i64data, n),
               (unsigned long long)sum_u32_into_u64(u32data, n));
    }
    printf("checksum %016llx\n", (unsigned long long)h);
    return 0;
}
