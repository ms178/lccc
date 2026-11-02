/* 256-bit deferred-store chain (AVX2 compare loop).
 * `acc = _mm256_xor_si256(acc, eq)` with eq used only by the adjacent
 * intrinsic (args[1]) exercises: (a) the deferred store + load-order swap
 * (eq must flow register-to-register via vmovdqa, NOT through its slot), and
 * (b) the non-escaping over-aligned alloca alignment downgrade (acc's slot
 * must be accessed directly, no lea/add/and dance). Before the fix, the AVX
 * loader's last-store peephole was gated on ymm0, so the args[1]-first load
 * into ymm1 reloaded the never-written eq slot -> garbage (movemask 0
 * instead of 0x7f for this input). */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>

uint64_t f(uint8_t *a, uint8_t *b, int n) {
    __m256i acc = _mm256_setzero_si256();
    for (int i = 0; i < n; i += 32) {
        __m256i x = _mm256_loadu_si256((const __m256i*)(a+i));
        __m256i y = _mm256_loadu_si256((const __m256i*)(b+i));
        __m256i eq = _mm256_cmpeq_epi8(x, y);
        acc = _mm256_xor_si256(acc, eq);
    }
    return (uint64_t)_mm256_movemask_epi8(acc);
}

int main(void) {
    uint8_t a[128], b[128];
    for (int i = 0; i < 128; i++) { a[i] = (uint8_t)i; b[i] = (uint8_t)(i % 7); }
    uint32_t ref = 0;
    for (int i = 0; i < 128; i += 32) {
        uint32_t eq = 0;
        for (int k = 0; k < 32; k++) if (a[i+k] == b[i+k]) eq |= 1u << k;
        ref ^= eq;
    }
    uint64_t got = f(a, b, 128);
    if (got != ref) { printf("FAIL got=%llx ref=%x\n", (unsigned long long)got, ref); return 1; }
    printf("OK simd_avx2_defer_chain\n");
    return 0;
}
