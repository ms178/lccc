/* Assembly-oracle witness for SV-01. Runtime/guard-page differentials live in
 * aos_pair_shapes.py. Distinct arrays keep the original non-aliasing shape;
 * both even and odd addresses must retain every selected SIB index root. */
extern float simd_x[128], simd_y[128], simd_o[128];

__attribute__((noinline)) void aos_crossadd(void) {
    for (int i = 0; i < 64; ++i) {
        simd_o[2*i] = simd_x[2*i] + simd_y[2*i+1];
        simd_o[2*i+1] = simd_x[2*i+1] + simd_y[2*i];
    }
}

__attribute__((noinline)) void aos_cmul(void) {
    for (int i = 0; i < 64; ++i) {
        float ar = simd_x[2*i], ai = simd_x[2*i+1];
        float br = simd_y[2*i], bi = simd_y[2*i+1];
        simd_o[2*i] = ar*br - ai*bi;
        simd_o[2*i+1] = ar*bi + ai*br;
    }
}
