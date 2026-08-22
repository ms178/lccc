/* __builtin_ia32_cvtpd2ps: raw GCC builtin, v2df -> v4sf BY VALUE.
 *
 * glibc sysdeps/x86/fpu/sincosf_poly.h (only C consumer in the tree):
 *
 *   v4sf_t v4sf = __builtin_ia32_cvtpd2ps (v2df);
 *   *f0p = v4sf[0];  *f1p = v4sf[1];
 *
 * Routed through X86IntrinsicKind::Vec128Value -> IntrinsicOp::CvtPd2Ps128
 * (the same emission the _mm_cvtpd_ps wrapper uses), so the result carries
 * value semantics and lane subscripts read real converted data. Without the
 * mapping, s_sincosf.os carried an undefined __builtin_ia32_cvtpd2ps and
 * libm.so could not link.
 */
#include <stdio.h>

typedef double v2df_t __attribute__((vector_size(2 * sizeof(double))));
typedef float v4sf_t __attribute__((vector_size(4 * sizeof(float))));

static inline void v2df_to_sf(v2df_t v2df, float *f0p, float *f1p)
{
    v4sf_t v4sf = __builtin_ia32_cvtpd2ps(v2df);
    *f0p = v4sf[0];
    *f1p = v4sf[1];
}

__attribute__((noinline)) void conv(double a, double b, float *x, float *y)
{
    v2df_t v = { a, b };
    v2df_to_sf(v, x, y);
}

int main(void)
{
    float x, y;
    conv(1.5, -2.25, &x, &y);
    int ok = (x == 1.5f) && (y == -2.25f);
    /* rounding on conversion: nearest float to a non-representable double */
    conv(0.1, 3.0e38, &x, &y);
    ok &= (x == (float)0.1) && (y == (float)3.0e38);
    /* upper lanes of cvtpd2ps are zeroed */
    v2df_t v = { 7.0, 8.0 };
    v4sf_t r = __builtin_ia32_cvtpd2ps(v);
    ok &= (r[2] == 0.0f) && (r[3] == 0.0f);
    printf("cvtpd2ps:%s %g %g\n", ok ? "ok" : "MISMATCH", x, y);
    return ok ? 0 : 1;
}
