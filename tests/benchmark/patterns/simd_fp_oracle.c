/*
 * SIMD/FP code-generation oracle corpus.
 *
 * Every externally-visible noinline function isolates one high-value loop or
 * basic-block shape.  There is deliberately no main(): scripts/godbolt.py and
 * scripts/codegen_scoreboard.py compare each function against GCC 16.2,
 * Clang, ICC and current ICX without benchmark-harness noise.
 *
 * Run two semantic modes:
 *   strict: -O3 -march=x86-64-v3
 *   fast:   -O3 -march=x86-64-v3 -ffast-math -ffp-contract=fast
 */
#define NOINLINE __attribute__((noinline))

/* 01-08: fundamental contiguous memory/arithmetic kernels. */
NOINLINE void p01_copy_f32(float *restrict d, const float *restrict a, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i];
}
NOINLINE void p02_copy_f64(double *restrict d, const double *restrict a, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i];
}
NOINLINE void p03_fill_f32(float *restrict d, float x, int n) {
    for (int i = 0; i < n; i++) d[i] = x;
}
NOINLINE void p04_add_f32(float *restrict d, const float *restrict a,
                          const float *restrict b, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] + b[i];
}
NOINLINE void p05_add_f64(double *restrict d, const double *restrict a,
                          const double *restrict b, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] + b[i];
}
NOINLINE void p06_sub_f32(float *restrict d, const float *restrict a,
                          const float *restrict b, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] - b[i];
}
NOINLINE void p07_mul_f32(float *restrict d, const float *restrict a,
                          const float *restrict b, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] * b[i];
}
NOINLINE void p08_div_f32(float *restrict d, const float *restrict a,
                          const float *restrict b, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] / b[i];
}

/* 09-14: FMA/affine maps and in-place streams. */
NOINLINE void p09_triad_f32(float *restrict d, const float *restrict a,
                            const float *restrict b, const float *restrict c, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] * b[i] + c[i];
}
NOINLINE void p10_triad_f64(double *restrict d, const double *restrict a,
                            const double *restrict b, const double *restrict c, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] * b[i] + c[i];
}
NOINLINE void p11_axpy_f32(float *restrict y, const float *restrict x, float a, int n) {
    for (int i = 0; i < n; i++) y[i] += a * x[i];
}
NOINLINE void p12_affine_f32(float *restrict d, const float *restrict a,
                             float scale, float bias, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] * scale + bias;
}
NOINLINE void p13_affine_i32(int *restrict d, const int *restrict a,
                             int scale, int bias, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] * scale + bias;
}
NOINLINE void p14_lerp_f32(float *restrict d, const float *restrict a,
                           const float *restrict b, float t, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] + t * (b[i] - a[i]);
}

/* 15-23: reduction families (strict and reassociation-enabled modes). */
NOINLINE float p15_sum_f32(const float *a, int n) {
    float s = 0.0f; for (int i = 0; i < n; i++) s += a[i]; return s;
}
NOINLINE double p16_sum_f64(const double *a, int n) {
    double s = 0.0; for (int i = 0; i < n; i++) s += a[i]; return s;
}
NOINLINE float p17_dot_f32(const float *a, const float *b, int n) {
    float s = 0.0f; for (int i = 0; i < n; i++) s += a[i] * b[i]; return s;
}
NOINLINE double p18_dot_f64(const double *a, const double *b, int n) {
    double s = 0.0; for (int i = 0; i < n; i++) s += a[i] * b[i]; return s;
}
NOINLINE int p19_sum_i32(const int *a, int n) {
    int s = 0; for (int i = 0; i < n; i++) s += a[i]; return s;
}
NOINLINE long p20_sum_i64(const long *a, int n) {
    long s = 0; for (int i = 0; i < n; i++) s += a[i]; return s;
}
NOINLINE unsigned long p21_sum_u8(const unsigned char *a, int n) {
    unsigned long s = 0; for (int i = 0; i < n; i++) s += a[i]; return s;
}
NOINLINE long p22_sum_i16(const short *a, int n) {
    long s = 0; for (int i = 0; i < n; i++) s += a[i]; return s;
}
NOINLINE float p23_sum_squares_f32(const float *a, int n) {
    float s = 0.0f; for (int i = 0; i < n; i++) s += a[i] * a[i]; return s;
}

/* 24-31: comparisons, masks, min/max, and classification-shaped loops. */
NOINLINE void p24_min_f32(float *restrict d, const float *restrict a,
                          const float *restrict b, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] < b[i] ? a[i] : b[i];
}
NOINLINE void p25_max_f32(float *restrict d, const float *restrict a,
                          const float *restrict b, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] > b[i] ? a[i] : b[i];
}
NOINLINE void p26_abs_f32(float *restrict d, const float *restrict a, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] < 0.0f ? -a[i] : a[i];
}
NOINLINE void p27_clamp_f32(float *restrict d, const float *restrict a,
                            float lo, float hi, int n) {
    for (int i = 0; i < n; i++) {
        float x = a[i]; if (x < lo) x = lo; if (x > hi) x = hi; d[i] = x;
    }
}
NOINLINE void p28_select_f32(float *restrict d, const float *restrict a,
                             const float *restrict b, const float *restrict c,
                             float threshold, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] > threshold ? b[i] : c[i];
}
NOINLINE int p29_count_gt_f32(const float *a, float threshold, int n) {
    int s = 0; for (int i = 0; i < n; i++) s += a[i] > threshold; return s;
}
NOINLINE float p30_conditional_sum_f32(const float *a, float threshold, int n) {
    float s = 0.0f; for (int i = 0; i < n; i++) if (a[i] > threshold) s += a[i]; return s;
}
NOINLINE void p31_sign_apply_f32(float *restrict d, const float *restrict a,
                                 const float *restrict sign, int n) {
    for (int i = 0; i < n; i++) d[i] = sign[i] < 0.0f ? -a[i] : a[i];
}

/* 24b-27d: exactness-critical min/max/select shapes.  These are the shapes
 * that must NOT fold to VMINPS/VMAXPS (swapped arms, <=) and the shapes
 * that MUST (nested ternary clamp, min/max chains, the F64 twins) — the
 * operand-order contract of MINPS/MAXPS (second source returned on
 * unordered and both-zero lanes) makes each fold a bit-exact decision. */
NOINLINE void p24b_min_swapped_f32(float *restrict d, const float *restrict a,
                                   const float *restrict b, int n) {
    /* `a < b ? b : a` is NOT max(a,b) for +-0 lanes: blendv lowering. */
    for (int i = 0; i < n; i++) d[i] = a[i] < b[i] ? b[i] : a[i];
}
NOINLINE void p24c_min_le_f32(float *restrict d, const float *restrict a,
                              const float *restrict b, int n) {
    /* `a <= b ? a : b` is NOT min(a,b) for +-0 lanes: blendv lowering. */
    for (int i = 0; i < n; i++) d[i] = a[i] <= b[i] ? a[i] : b[i];
}
NOINLINE void p27b_clamp_ternary_f32(float *restrict d, const float *restrict a,
                                     int n) {
    for (int i = 0; i < n; i++)
        d[i] = a[i] < 0.0f ? 0.0f : (a[i] > 1.0f ? 1.0f : a[i]);
}
NOINLINE void p27c_minmax_chain_f32(float *restrict d, const float *restrict a,
                                    int n) {
    for (int i = 0; i < n; i++) {
        float x = a[i] < 0.0f ? 0.0f : a[i];
        d[i] = x > 1.0f ? 1.0f : x;
    }
}
NOINLINE void p24d_min_f64(double *restrict d, const double *restrict a,
                           const double *restrict b, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] < b[i] ? a[i] : b[i];
}
NOINLINE void p27d_clamp_f64(double *restrict d, const double *restrict a,
                             int n) {
    for (int i = 0; i < n; i++) {
        double x = a[i];
        if (x < 0.0) x = 0.0;
        else if (x > 1.0) x = 1.0;
        d[i] = x;
    }
}

/* 32-37: expensive FP operations and algebraic instruction selection. */
NOINLINE void p32_sqrt_f32(float *restrict d, const float *restrict a, int n) {
    for (int i = 0; i < n; i++) d[i] = __builtin_sqrtf(a[i]);
}
NOINLINE void p33_sqrt_f64(double *restrict d, const double *restrict a, int n) {
    for (int i = 0; i < n; i++) d[i] = __builtin_sqrt(a[i]);
}
NOINLINE void p34_reciprocal_f32(float *restrict d, const float *restrict a, int n) {
    for (int i = 0; i < n; i++) d[i] = 1.0f / a[i];
}
NOINLINE void p35_poly3_f32(float *restrict d, const float *restrict x,
                            float a, float b, float c, float e, int n) {
    for (int i = 0; i < n; i++) d[i] = ((a * x[i] + b) * x[i] + c) * x[i] + e;
}
NOINLINE void p36_mul_const_f32(float *restrict d, const float *restrict a, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] * 0.125f;
}
NOINLINE void p37_div_const_f32(float *restrict d, const float *restrict a, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] / 8.0f;
}

/* 38-44: layout/dependence patterns from media, codecs, and numerical code. */
NOINLINE void p38_stencil3_f32(float *restrict d, const float *restrict a, int n) {
    for (int i = 1; i + 1 < n; i++) d[i] = a[i - 1] + 2.0f * a[i] + a[i + 1];
}
NOINLINE void p39_stencil5_f32(float *restrict d, const float *restrict a, int n) {
    for (int i = 2; i + 2 < n; i++)
        d[i] = a[i - 2] + a[i - 1] + a[i] + a[i + 1] + a[i + 2];
}
NOINLINE void p40_complex_mul_f32(float *restrict d, const float *restrict a,
                                  const float *restrict b, int n) {
    for (int i = 0; i < n; i++) {
        float ar = a[2*i], ai = a[2*i+1], br = b[2*i], bi = b[2*i+1];
        d[2*i] = ar*br - ai*bi; d[2*i+1] = ar*bi + ai*br;
    }
}
NOINLINE float p41_complex_norm_f32(const float *a, int n) {
    float s = 0.0f;
    for (int i = 0; i < n; i++) s += a[2*i]*a[2*i] + a[2*i+1]*a[2*i+1];
    return s;
}
NOINLINE float p42_stride2_sum_f32(const float *a, int n) {
    float s = 0.0f; for (int i = 0; i < n; i++) s += a[2*i]; return s;
}
NOINLINE float p43_gather_sum_f32(const float *a, const int *index, int n) {
    float s = 0.0f; for (int i = 0; i < n; i++) s += a[index[i]]; return s;
}
NOINLINE void p44_rgba_to_gray_f32(float *restrict d, const float *restrict rgba, int n) {
    for (int i = 0; i < n; i++)
        d[i] = rgba[4*i] * 0.299f + rgba[4*i+1] * 0.587f + rgba[4*i+2] * 0.114f;
}

/* 45-53: conversion, independent accumulators, recurrences, fixed distances. */
NOINLINE void p45_u8_to_f32(float *restrict d, const unsigned char *restrict a, int n) {
    for (int i = 0; i < n; i++) d[i] = (float)a[i];
}
NOINLINE void p46_i16_to_f32(float *restrict d, const short *restrict a, int n) {
    for (int i = 0; i < n; i++) d[i] = (float)a[i];
}
NOINLINE void p47_f32_to_i32(int *restrict d, const float *restrict a, int n) {
    for (int i = 0; i < n; i++) d[i] = (int)a[i];
}
NOINLINE void p48_four_sums_f32(const float *a, int n, float *out) {
    float s0=0.0f, s1=0.0f, s2=0.0f, s3=0.0f;
    for (int i = 0; i < n; i++) {
        s0 += a[4*i]; s1 += a[4*i+1]; s2 += a[4*i+2]; s3 += a[4*i+3];
    }
    out[0]=s0; out[1]=s1; out[2]=s2; out[3]=s3;
}
NOINLINE unsigned int p49_adler_chunk(const unsigned char *a, int n) {
    unsigned int s1 = 1, s2 = 0;
    for (int i = 0; i < n; i++) { s1 += a[i]; s2 += s1; }
    return (s2 << 16) | (s1 & 65535U);
}
NOINLINE double p50_distance3_f64(const double *a, const double *b) {
    double x=a[0]-b[0], y=a[1]-b[1], z=a[2]-b[2];
    return x*x + y*y + z*z;
}
NOINLINE float p51_distance4_f32(const float *a, const float *b) {
    float x0=a[0]-b[0], x1=a[1]-b[1], x2=a[2]-b[2], x3=a[3]-b[3];
    return x0*x0 + x1*x1 + x2*x2 + x3*x3;
}
NOINLINE double p52_distance4_f64(const double *a, const double *b) {
    double x0=a[0]-b[0], x1=a[1]-b[1], x2=a[2]-b[2], x3=a[3]-b[3];
    return x0*x0 + x1*x1 + x2*x2 + x3*x3;
}
NOINLINE float p53_distance8_f32(const float *a, const float *b) {
    float x0=a[0]-b[0], x1=a[1]-b[1], x2=a[2]-b[2], x3=a[3]-b[3];
    float x4=a[4]-b[4], x5=a[5]-b[5], x6=a[6]-b[6], x7=a[7]-b[7];
    return x0*x0 + x1*x1 + x2*x2 + x3*x3
         + x4*x4 + x5*x5 + x6*x6 + x7*x7;
}
