/* Original non-kernel stress fixture for explicit AVX FP temporary homes.
 * Eight independent clamps must not reserve permanent over-aligned scratch
 * objects for every min/max. Every output lane remains observable. */
typedef float vf8 __attribute__((vector_size(32)));
union bits8 { vf8 v; unsigned u[8]; };
static __attribute__((always_inline)) inline vf8 clamp(vf8 x, vf8 lo, vf8 hi) {
    return __builtin_ia32_maxps256(__builtin_ia32_minps256(x, hi), lo);
}
__attribute__((noinline)) void avx_private_frame(vf8 *out, const vf8 *in) {
    vf8 lo = {0,0,0,0,0,0,0,0};
    vf8 hi = {1,1,1,1,1,1,1,1};
    out[0] = clamp(in[0], lo, hi);
    out[1] = clamp(in[1], lo, hi);
    out[2] = clamp(in[2], lo, hi);
    out[3] = clamp(in[3], lo, hi);
    out[4] = clamp(in[4], lo, hi);
    out[5] = clamp(in[5], lo, hi);
    out[6] = clamp(in[6], lo, hi);
    out[7] = clamp(in[7], lo, hi);
}
int main(void) {
    vf8 input[8], output[8];
    const unsigned values[8] = {0xff800000u,0xc0000000u,0x80000000u,0,
                                0x3f000000u,0x3f800000u,0x40000000u,0x7fc00123u};
    const unsigned expected[8] = {0,0,0,0,0x3f000000u,0x3f800000u,0x3f800000u,0x3f800000u};
    for (int i = 0; i < 8; ++i) {
        union bits8 x;
        for (int j = 0; j < 8; ++j) x.u[j] = values[(i+j)&7];
        input[i] = x.v;
    }
    avx_private_frame(output, input);
    for (int i = 0; i < 8; ++i) {
        union bits8 x; x.v = output[i];
        for (int j = 0; j < 8; ++j)
            if (x.u[j] != expected[(i+j)&7]) return 1+i*8+j;
    }
    return 0;
}
