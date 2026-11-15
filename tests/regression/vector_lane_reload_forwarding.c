/* Narrow reloads of a just-homed vector register must read the lane the
 * scalar load addressed — not lane 0, not the neighbouring lane, and with
 * the scalar load's exact zeroing semantics.
 *
 * The x86 peephole `vector_lane_reload` rewrites
 *
 *     vmovdqu %ymm0, 8(%rsp)          # parameter homed to the frame
 *     vmovss 20(%rsp), %xmm2          # constant-index lane read back
 *
 * into a register-only lane extraction (`vinsertps`/`vmovq`/`vpsrldq`/
 * `(v)extractps`/`(v)pextrq`), removing the store-to-load round trip that
 * GCC 16.2 never emits for these shapes.  Every rewrite recomputes the
 * lane from the reload's displacement, so an off-by-one in the immediate
 * (count_s is bits 7:6 of the insertps immediate, not 1:0), a swapped
 * pextrq index, or a byte-shift of the wrong width all surface here as a
 * wrong element value.
 *
 * The float/double cases return through %xmm0, so a replacement that
 * failed to zero the destination's upper lanes the way the memory-form
 * scalar load does would also corrupt later vector reads of the return
 * register; g() feeding h() below checks that path.
 */
#include <immintrin.h>
#include <stdio.h>

__attribute__((noinline)) float lane3(__m256 a) { return a[3]; }
__attribute__((noinline)) float lane7(__m256 a) { return a[7]; }
__attribute__((noinline)) float lane0(__m128 a) { return a[0]; }
__attribute__((noinline)) double dlane1(__m128d a) { return a[1]; }
__attribute__((noinline)) int ilane2(__m128i a) { return ((int *)&a)[2]; }
__attribute__((noinline)) long qlane1(__m128i a) { return ((long *)&a)[1]; }

/* The scalar result of a forwarded lane read, used AS a vector: the
 * replacement must have zeroed everything above bits 31:0 exactly like
 * the `vmovss` it replaced, or the horizontal sum changes. */
__attribute__((noinline)) float sum_after_extract(__m256 a) {
    __m128 s = _mm_set_ss(lane3(a));
    __m128 t = _mm_add_ps(s, s);
    float out[4];
    _mm_storeu_ps(out, t);
    return out[0] + out[1] + out[2] + out[3];
}

int main(void) {
    __m256 a;
    float *ap = (float *)&a;
    for (int k = 0; k < 8; k++)
        ap[k] = 1.5f + (float)k;

    __m128 b;
    float *bp = (float *)&b;
    for (int k = 0; k < 4; k++)
        bp[k] = -2.0f * (float)k - 0.25f;

    __m128d d;
    double *dp = (double *)&d;
    dp[0] = -1.25;
    dp[1] = 9.75;

    __m128i iv;
    int *ip = (int *)&iv;
    ip[0] = 10;
    ip[1] = 20;
    ip[2] = 30;
    ip[3] = 40;

    __m128i qv;
    long *qp = (long *)&qv;
    qp[0] = 1L;
    qp[1] = 0x1122334455667788L;

    int bad = 0;
    if (lane3(a) != 4.5f) bad |= 1;
    if (lane7(a) != 8.5f) bad |= 2;
    if (lane0(b) != -0.25f) bad |= 4;
    if (dlane1(d) != 9.75) bad |= 8;
    if (ilane2(iv) != 30) bad |= 16;
    if (qlane1(qv) != 0x1122334455667788L) bad |= 32;
    if (sum_after_extract(a) != 9.0f) bad |= 64;

    printf("%g %g %g %g %d %ld %g bad=%d\n", (double)lane3(a),
           (double)lane7(a), (double)lane0(b), dlane1(d), ilane2(iv),
           qlane1(qv), (double)sum_after_extract(a), bad);
    return bad;
}
