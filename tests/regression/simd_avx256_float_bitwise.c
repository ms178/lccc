/*
 * AVX 256-bit float bitwise operations.
 *
 * `_mm256_and/or/xor/andnot_(ps|pd)` are lane-agnostic bitwise ops: on x86
 * each is ONE instruction (`vandps`/`vandpd`/`vorps`/.../`vandnps`), and the
 * float spelling is a pure reinterpretation of the integer form.  The header
 * used to implement the `pd` forms as element-wise `union` loops and did not
 * provide the `ps` forms at all; the loop bodies lowered to a per-lane GPR
 * round trip (`vmovsd`/`vmovq`/`xor`/`vmovsd`, four times) because the
 * bit-fiddle is opaque to the SLP vectorizer.
 *
 * This fixture is the executed half of
 * `tests/regression/check_avx256_float_bitwise.sh`; the shell half asserts the
 * emitted shape.  The values are chosen so that every operation has a
 * *distinguishable* result per lane and per operation: an implementation that
 * swapped AND with OR, dropped a lane, or used the wrong operand order for
 * `andnot` (which is `~a & b`, not `a & ~b`) prints a different line.
 */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

static double da[4] = {1.5, -2.25, 3.0, -0.0};
static double db[4] = {2.0, 3.0, -4.5, 0.0};
/* `loop_mix` reads up to 16 doubles (n <= 16); a shorter object would make the
 * fixture read out of bounds, which is undefined and would compare two
 * compilers' stack garbage instead of their bitwise lowering. */
static double loopbuf[16] = {1.5, -2.25, 3.0, -0.0, 0.5, 6.25, -7.0, 8.0,
                             9.5, -10.5, 11.0, 12.25, -13.0, 14.0, 15.5, -16.0};
static float fa[8] = {1.0f, -2.0f, 3.0f, -4.0f, 5.0f, -6.0f, 7.0f, -8.0f};
static float fb[8] = {8.0f, 7.0f, -6.0f, 5.0f, -4.0f, 3.0f, -2.0f, 1.0f};
static int64_t ia[4] = {(int64_t)0xff00ff00ff00ff00ULL, 0x0f0f0f0f0f0f0f0fLL,
                        -1LL, 0x0123456789abcdefLL};
static int64_t ib[4] = {(int64_t)0x00ff00ff00ff00ffULL, (int64_t)0xf0f0f0f0f0f0f0f0ULL,
                        7LL, 0x0f0f0f0f0f0f0f0fLL};

/* Print the raw bits so a lane swap, a sign-bit change or a mis-ordered
 * andnot is visible; printing the doubles would hide NaN payload differences
 * and could print "-nan" identically. */
static void show(const char *tag, const void *p, int nbytes) {
    const unsigned char *b = (const unsigned char *)p;
    printf("%s", tag);
    for (int i = 0; i < nbytes; i++) printf(" %02x", b[i]);
    printf("\n");
}

__attribute__((noinline)) static void pd_ops(void) {
    __m256d a = _mm256_loadu_pd(da);
    __m256d b = _mm256_loadu_pd(db);
    double out[4];
    _mm256_storeu_pd(out, _mm256_and_pd(a, b));
    show("and_pd", out, sizeof out);
    _mm256_storeu_pd(out, _mm256_or_pd(a, b));
    show("or_pd", out, sizeof out);
    _mm256_storeu_pd(out, _mm256_xor_pd(a, b));
    show("xor_pd", out, sizeof out);
    _mm256_storeu_pd(out, _mm256_andnot_pd(a, b));
    show("andnot_pd", out, sizeof out);
}

__attribute__((noinline)) static void ps_ops(void) {
    __m256 a = _mm256_loadu_ps(fa);
    __m256 b = _mm256_loadu_ps(fb);
    float out[8];
    _mm256_storeu_ps(out, _mm256_and_ps(a, b));
    show("and_ps", out, sizeof out);
    _mm256_storeu_ps(out, _mm256_or_ps(a, b));
    show("or_ps", out, sizeof out);
    _mm256_storeu_ps(out, _mm256_xor_ps(a, b));
    show("xor_ps", out, sizeof out);
    _mm256_storeu_ps(out, _mm256_andnot_ps(a, b));
    show("andnot_ps", out, sizeof out);
}

__attribute__((noinline)) static void si_ops(void) {
    __m256i a = _mm256_loadu_si256((const __m256i *)ia);
    __m256i b = _mm256_loadu_si256((const __m256i *)ib);
    int64_t out[4];
    _mm256_storeu_si256((__m256i *)out, _mm256_and_si256(a, b));
    show("and_si256", out, sizeof out);
    _mm256_storeu_si256((__m256i *)out, _mm256_or_si256(a, b));
    show("or_si256", out, sizeof out);
    _mm256_storeu_si256((__m256i *)out, _mm256_xor_si256(a, b));
    show("xor_si256", out, sizeof out);
    _mm256_storeu_si256((__m256i *)out, _mm256_andnot_si256(a, b));
    show("andnot_si256", out, sizeof out);
}

/* A loop keeps the ops in a hot context where a scalarized lowering would be
 * a real cost rather than a one-off.  The running value is data-dependent, so
 * constant folding cannot remove the operations. */
__attribute__((noinline)) static uint64_t loop_mix(const double *p, long n,
                                                   __m256d mask) {
    __m256d acc = _mm256_setzero_pd();
    for (long i = 0; i + 4 <= n; i += 4) {
        __m256d v = _mm256_loadu_pd(p + i);
        acc = _mm256_xor_pd(acc, _mm256_and_pd(v, mask));
        acc = _mm256_or_pd(acc, _mm256_andnot_pd(mask, v));
    }
    double lanes[4];
    _mm256_storeu_pd(lanes, acc);
    uint64_t h = 0;
    for (int i = 0; i < 4; i++) {
        uint64_t bits;
        memcpy(&bits, &lanes[i], sizeof bits);
        h = h * 31 + bits;
    }
    return h;
}

int main(void) {
    pd_ops();
    ps_ops();
    si_ops();

    __m256d mask = _mm256_castsi256_pd(_mm256_set1_epi64x((long long)0xff00ff00ff00ff00ULL));
    uint64_t h = 0;
    for (long n = 0; n <= 16; n++) h = h * 131 + loop_mix(loopbuf, n, mask);
    printf("loop %016llx\n", (unsigned long long)h);
    return 0;
}
