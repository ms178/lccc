/* Round-8 "widen-then-average" rounding-mode proof.
 *
 * The ISPC `ImproveMemoryOps` / `InstructionSimplify` transplant adds a
 * byte-local `pavgb` rule to the constant-image lattice:
 *
 *     avg(a, b) = (a + b + 1) >> 1     computed in a WIDER u16 intermediate
 *
 * That widening is the whole point: `0xFF + 0xFF + 1` is 511, not 255, so a
 * u8 intermediate would wrap and produce 0x7F instead of 0xFF.  The rule is
 * only sound if EVERY byte pair agrees with the hardware rounding (round
 * half UP), on every backend.  The previous audit listed "no rounding-mode
 * proof" as the blocker for this follow-up; this fixture is that proof, run
 * against GCC 16.2 and Clang 23.1.5 by `run_regression.py`.
 *
 * It uses only SSE2 (`_mm_avg_epu8`), so it executes on EVERY x86-64 hosted
 * runner -- no AVX-512 requirement, no SIGILL risk.  All 65,536 byte pairs
 * are checked; the accumulated FNV-1a checksum is compared with the oracle
 * compiler's stdout.
 */
#include <stdio.h>
#include <emmintrin.h>

int main(void) {
    unsigned long long acc = 1469598103934665603ULL;
    for (int x = 0; x < 256; x++) {
        for (int y = 0; y < 256; y++) {
            __m128i va = _mm_set1_epi8((char)x);
            __m128i vb = _mm_set1_epi8((char)y);
            __m128i vr = _mm_avg_epu8(va, vb);
            unsigned got = (unsigned)_mm_cvtsi128_si32(vr) & 0xFFu;
            unsigned want = (unsigned)((x + y + 1) >> 1);
            if (got != want) {
                printf("MISMATCH x=%d y=%d got=%u want=%u\n", x, y, got, want);
                return 1;
            }
            acc = (acc ^ got) * 1099511628211ULL;
        }
    }
    printf("%016llx\n", acc);
    return 0;
}
