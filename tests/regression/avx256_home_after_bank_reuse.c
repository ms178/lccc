/* Regression: a 256-bit value stored from a YMM register must not be read
 * back from that register after another value has claimed it.
 *
 * `t10` is stored from `%ymm15` into slot `s` (a whole-register store records
 * a home: slot `s` is resident in `%ymm15`). The next permute, `t11`, claims
 * `%ymm15` too. A later read of `s` (the unpackhi through the slot) must see
 * `t10`, not `t11`. Before the fix the home record survived the claim, so the
 * read took `%ymm15` and returned `t11`. Differs from GCC 16.2 at `-O3`
 * (fuzz seed 1234, case 137, reduced by reduce.py).
 */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>

__attribute__((noinline)) static __m256i helper(__m256i a, __m256i b) {
    return _mm256_xor_si256(_mm256_add_epi64(a, b), _mm256_set1_epi64x(0x9e3779b97f4a7c15LL));
}

__attribute__((noinline)) void kernel(const uint64_t *in, uint64_t *out, long n) {
    __m256i a1 = _mm256_loadu_si256((const __m256i *)(in + 4));
    __m256i a2 = _mm256_loadu_si256((const __m256i *)(in + 8));
    __m256i a3 = _mm256_loadu_si256((const __m256i *)(in + 12));
    __m256i a4 = _mm256_loadu_si256((const __m256i *)(in + 16));
    __m256i a5 = _mm256_loadu_si256((const __m256i *)(in + 20));
    __m256i p0 = _mm256_set1_epi64x(385626484);
    __m256i p1 = _mm256_set1_epi64x(964590987);
    for (long it = 0; it < n; it++) {
        __m256i x = _mm256_loadu_si256((const __m256i *)(in + 4 * ((it * 3) & 15)));
        if ((_mm256_movemask_epi8(_mm256_cmpgt_epi64(a3, p0)) >> 16) & 1) {
            if ((_mm256_movemask_epi8(_mm256_cmpgt_epi64(p0, p1)) >> 24) & 1) {
            }
        }
        __m256i t10 = _mm256_permute4x64_epi64(a4, 41);
        __m256i t11 = _mm256_permute4x64_epi64(a2, 137);
        __m256i t12 = _mm256_unpackhi_epi64(a4, t10);
        __m256i t14 = _mm256_srli_epi64(p0, 52);
        if ((_mm256_movemask_epi8(_mm256_cmpgt_epi64(x, t14)) >> 24) & 1) {
            __m256i t22 = _mm256_slli_epi64(t12, 17);
            a1 = t22;
        }
        if ((_mm256_movemask_epi8(_mm256_cmpgt_epi64(a5, t12)) >> 0) & 1) {
        }
        a1 = helper(a1, t11);
    }
    _mm256_storeu_si256((__m256i *)(out + 4), a1);
}

int main(void) {
    uint64_t seed = 1;
    uint64_t in[64] __attribute__((aligned(32)));
    uint64_t out[64] __attribute__((aligned(32)));
    static const long counts[] = {0, 1, 2, 3, 7, 16};
    for (unsigned round = 0; round < 6; round++) {
        for (int i = 0; i < 64; i++) {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            in[i] = seed;
        }
        for (int i = 0; i < 64; i++) out[i] = 0x5555555555555555ull;
        kernel(in, out, counts[round]);
        for (int i = 0; i < 64; i++) printf("%016llx\n", (unsigned long long)out[i]);
    }
    return 0;
}
