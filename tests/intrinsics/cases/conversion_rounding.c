// FLAGS: -msse2
/* CVT observes MXCSR; CVTT always truncates. Preserve the caller's MXCSR. */
#include <immintrin.h>
#include <stdio.h>
static volatile float input_f;
static volatile double input_d;
int main(void) {
    unsigned saved = _mm_getcsr();
    for (unsigned mode = 0; mode < 4; ++mode) {
        _mm_setcsr((saved & ~0x6000u) | (mode << 13));
        for (int sign = -1; sign <= 1; sign += 2) {
            input_f = sign * 3.75f;
            input_d = sign * 3.75;
            __m128 s = _mm_set_ss(input_f);
            __m128d d = _mm_set_sd(input_d);
            printf("%u %d %d %d %d\n", mode, _mm_cvtss_si32(s), _mm_cvttss_si32(s),
                   _mm_cvtsd_si32(d), _mm_cvttsd_si32(d));
        }
    }
    _mm_setcsr(saved);
    return 0;
}
