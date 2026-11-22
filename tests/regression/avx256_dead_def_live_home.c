/* Regression: a dead vector definition must not share the YMM home of a value
 * that is live across it.
 *
 * `v` (the vpermq result) is live-in only to the else block of a diamond: the
 * then block does not read it, so its live range has a hole there and restarts
 * at the else entry. The else block's first producer, `vpshufb`, has no reader.
 * Its instruction still writes its destination at that entry point, which is
 * exactly where `v` begins to be live. The allocator used to give the dead
 * result a zero-length interval that overlapped nothing, so it shared `v`'s
 * home and the dead `vpshufb` overwrote `v` before the store that reads it.
 * Measured on the pre-fix build (69d11d6): `-O3` output differs from GCC 16.2 at
 * line 65 (round two). Root-cause fix: the allocator now gives a zero-length
 * definition a point-sized interval, so it conflicts with the live range it
 * lands in. Defense in depth: DCE also removes the now-unused register-only
 * 256-bit producer (`IntrinsicOp::is_pure`), so the dead write is gone.
 */
#include <immintrin.h>
#include <stdint.h>
__attribute__((noinline)) static __m256i helper(__m256i a, __m256i b) {
    return _mm256_xor_si256(_mm256_add_epi64(a, b), _mm256_set1_epi64x(0x9e3779b97f4a7c15LL));
}
__attribute__((noinline)) void kernel(const uint64_t *in, uint64_t *out, long n) {
    __m256i a0 = _mm256_loadu_si256((const __m256i *)(in + 0));
    __m256i a1 = _mm256_loadu_si256((const __m256i *)(in + 4));
    __m256i a2 = _mm256_loadu_si256((const __m256i *)(in + 8));
    __m256i p0 = _mm256_set1_epi64x(160957279);
    __m256i p1 = _mm256_set1_epi64x(137799105);
    __m256i p2 = _mm256_set1_epi64x(213978323);
    __m256i p3 = _mm256_set1_epi64x(652153392);
    for (long it = 0; it < n; it++) {
        __m256i x = _mm256_loadu_si256((const __m256i *)(in + 4 * ((it * 3) & 15)));
        __m256i t5 = _mm256_slli_epi64(a2, 22);
        __m256i t6 = _mm256_shuffle_epi32(p0, 203);
        __m256i t7 = _mm256_shuffle_epi32(t6, 57);
        __m256i t8 = _mm256_permute4x64_epi64(a0, 184);
        __m256i t9 = _mm256_slli_epi64(t8, 25);
        __m256i t10 = _mm256_inserti128_si256(p1, _mm256_castsi256_si128(x), 0);
        __m256i t11 = _mm256_cmpgt_epi64(a2, t7);
        __m256i t14 = _mm256_permute4x64_epi64(p0, 98);
        __m256i t15 = _mm256_cmpgt_epi64(a2, t10);
        if ((_mm256_movemask_epi8(_mm256_cmpgt_epi64(p2, x)) >> 8) & 1) {
        }
        __m256i t23 = _mm256_and_si256(p3, t8);
        __m256i t24 = helper(t11, t5);
        __m256i t25 = _mm256_xor_si256(a1, t9);
        __m256i t26 = _mm256_blend_epi32(t24, t6, 135);
        __m256i t28 = _mm256_xor_si256(t24, t11);
        __m256i t29 = _mm256_slli_epi64(t14, 45);
        if ((_mm256_movemask_epi8(_mm256_cmpgt_epi64(t29, t10)) >> 16) & 1) {
            if ((_mm256_movemask_epi8(_mm256_cmpgt_epi64(t8, p0)) >> 16) & 1) {
            }
        }
        __m256i t46 = _mm256_inserti128_si256(t25, _mm256_castsi256_si128(t10), 0);
        __m256i t48 = _mm256_srli_epi64(a0, 39);
        __m256i t51 = _mm256_shuffle_epi32(t46, 113);
        if ((_mm256_movemask_epi8(_mm256_cmpgt_epi64(t26, t23)) >> 8) & 1) {
        }
        if ((_mm256_movemask_epi8(_mm256_cmpgt_epi64(t7, t15)) >> 24) & 1) {
            a2 = t14;
        } else {
            __m256i t64 = _mm256_shuffle_epi8(a2, t48);
            a0 = t51;
        }
        a0 = helper(a0, t28);
    }
    _mm256_storeu_si256((__m256i *)(out + 0), a0);
}

#include <stdio.h>

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
