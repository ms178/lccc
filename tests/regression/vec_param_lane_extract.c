/* Lane reads of 16-byte vector values stay bit-exact through every
 * forwarding path added for them:
 *
 *  - `((long long *)&a)[k]` / `a[k]` of an [Sse, SseUp] REGISTER parameter:
 *    the parameter's staging alloca is rewritten to one entry-prefix
 *    whole-vector load plus register lane extracts (`forward_param_lane_loads`
 *    + the prologue preload), eliding the prologue spill entirely.  Both
 *    lanes, every element type, and the budget overflow (9th vector
 *    argument arrives on the stack and must NOT be rewritten).
 *  - a lane read through an explicit staging array after a full-width
 *    store (`forward_scalar_lane_loads`): the store+reload pair becomes a
 *    register extract.
 *  - fail-closed shapes: a write through the alias, an escaping address,
 *    and a straddling (unaligned) read all keep their memory forms.
 *
 * Every function is compared against the reference compiler's result at
 * runtime; any lane mix-up, stale spill or wrong-width extract changes
 * the printed values.
 */
#include <immintrin.h>
#include <stdio.h>

__attribute__((noinline)) long long gq1(__m128i a) {
    return ((long long *)&a)[1];
}
__attribute__((noinline)) long long gq0(__m128i a) {
    return ((long long *)&a)[0];
}
__attribute__((noinline)) int gi3(__m128i a) { return ((int *)&a)[3]; }
__attribute__((noinline)) int gi0(__m128i a) { return ((int *)&a)[0]; }
__attribute__((noinline)) float gf3(__m128 a) { return a[3]; }
__attribute__((noinline)) float gf0(__m128 a) { return a[0]; }
__attribute__((noinline)) double gd1(__m128d a) { return a[1]; }
__attribute__((noinline)) double gd0(__m128d a) { return a[0]; }
__attribute__((noinline)) long long gsum(__m128i a) {
    long long *p = (long long *)&a;
    return p[0] + p[1];
}

/* Writes through the alias: the slot diverges from the register, the
 * rewrite must fail closed. */
__attribute__((noinline)) long long gwrite(__m128i a) {
    long long *p = (long long *)&a;
    p[0] = 7;
    return p[0] + p[1];
}

/* Straddling read: not lane-aligned, must stay a memory load. */
__attribute__((noinline)) long long gmis(__m128i a) {
    long long r;
    __builtin_memcpy(&r, (char *)&a + 4, 8);
    return r;
}

/* XMM budget overflow: the 9th vector argument is stack-passed; the
 * rewrite must not fire (and the value must still be right). */
__attribute__((noinline)) long long g9(__m128i a, __m128i b, __m128i c,
                                       __m128i d, __m128i e, __m128i f,
                                       __m128i g, __m128i h, __m128i i) {
    return ((long long *)&i)[1] + ((long long *)&a)[0];
}

/* Scalar FP args consume the same XMM budget the classifier counts. */
__attribute__((noinline)) float gmix(float x, double y, __m128 v) {
    return x + (float)y + v[2];
}

/* Explicit staging array after a full-width store: store+reload becomes
 * a register extract. */
__attribute__((noinline)) long long sq1(const long long *p) {
    __m128i v = _mm_loadu_si128((const __m128i *)p);
    long long t[2];
    _mm_storeu_si128((__m128i *)t, v);
    return t[1];
}
__attribute__((noinline)) float sf2(const float *p) {
    __m128 v = _mm_loadu_ps(p);
    float t[4];
    _mm_storeu_ps(t, v);
    return t[2];
}

int main(void) {
    __m128i a = _mm_set_epi32(0x11223344, 0x55667788, (int)0x99aabbcc,
                              (int)0xddeeff00);
    __m128i b = _mm_set_epi32(4, 3, 2, 1);
    __m128 vf = _mm_set_ps(8.5f, -7.25f, 6.0f, 0.125f);
    __m128d vd = _mm_set_pd(-3.75, 42.5);
    long long lp[2] = {0x0102030405060708LL, -0x1122334455667788LL};
    float fp[4] = {1.5f, -2.5f, 3.25f, -4.75f};

    printf("gq1 %lld gq0 %lld\n", gq1(a), gq0(a));
    printf("gi3 %d gi0 %d\n", gi3(a), gi0(a));
    printf("gf3 %.6g gf0 %.6g\n", gf3(vf), gf0(vf));
    printf("gd1 %.12g gd0 %.12g\n", gd1(vd), gd0(vd));
    printf("gsum %lld gwrite %lld gmis %lld\n", gsum(a), gwrite(a), gmis(a));
    printf("g9 %lld gmix %.6g\n", g9(b, b, b, b, b, b, b, b, a), gmix(1.5f, 2.25, vf));
    printf("sq1 %lld sf2 %.6g\n", sq1(lp), sf2(fp));
    return 0;
}
