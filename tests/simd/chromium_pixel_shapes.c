/* Reduced C witnesses of pinned Chromium integer pixel paths, not a browser
 * benchmark. Provenance: Chromium 042b7bd11fd2b37f7d8b445781e6d858d3f56554,
 * skia/ext/convolver.cc (BSD license, Chromium Authors). The convolution
 * accumulation is intentionally outside the witness: inputs are its four
 * signed accumulators, so no unproved signed-overflow assumption is needed.
 */
typedef unsigned char u8;
typedef unsigned int u32;
static u8 clamp_fast(int a) {
    if ((unsigned int)a < 256U) return a;
    if (a < 0) return 0;
    return 255;
}
static int clamp_simple(int a) {
    a = a > 0 ? a : 0;
    return a < 255 ? a : 255;
}
static int max(int a, int b) { return a > b ? a : b; }
void chromium_convolve_tail(const int *a, u8 *out) {
    u8 b = clamp_fast(a[0] >> 14), g = clamp_fast(a[1] >> 14);
    u8 r = clamp_fast(a[2] >> 14), alpha = clamp_fast(a[3] >> 14);
    out[0] = b; out[1] = g; out[2] = r;
    int rgb = max(out[0], max(out[1], out[2]));
    out[3] = alpha < rgb ? rgb : alpha;
}
void chromium_convolve_canonical(const int *a, u8 *out) {
    int b = clamp_simple(a[0] >> 14), g = clamp_simple(a[1] >> 14);
    int r = clamp_simple(a[2] >> 14), alpha = clamp_simple(a[3] >> 14);
    out[0] = b; out[1] = g; out[2] = r;
    out[3] = max(alpha, max(b, max(g, r)));
}
/* libyuv at the revision pinned by Chromium DEPS:
 * 1e2e3a35993609da1d89ac170b907719d555f4b3, source/row_common.cc.
 * Three-byte input/four-byte output expansion is a separate tail opportunity;
 * it has no arithmetic reduction and must not be given fake reduction credit.
 */
void chromium_rgb24_expand(const u8 *src, u8 *dst, int n) {
    for (int i = 0; i < n; ++i) {
        dst[4*i] = src[3*i]; dst[4*i+1] = src[3*i+1];
        dst[4*i+2] = src[3*i+2]; dst[4*i+3] = 255;
    }
}
