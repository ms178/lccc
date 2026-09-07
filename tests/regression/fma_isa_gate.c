#include <stdio.h>

/*
 * FMA3 ISA gate corpus (companion to vectorize_isa_gate.c; the emission
 * assertions live in tests/regression/check_vectorize_isa_gate.sh).
 *
 * `simplify::set_has_fma3` used to hard-code `true` for x86-64, so the
 * fma/fmaf libcall fold emitted `vfmadd*` even for a TU compiled with
 * `-mno-sse` or `-mno-avx`. `vfmadd*` is VEX-encoded and needs both AVX and
 * FMA3, so in the kernel (CR4.OSFXSR=0) that is an immediate #UD.
 *
 * The fold itself must keep firing by default: LCCC's x86-64 code-generation
 * baseline is x86-64-v3, and the fused form is what gives fmaf() its C99
 * single-rounding semantics.
 */
float fused(float a, float b, float c) { return __builtin_fmaf(a, b, c); }

double fused_d(double a, double b, double c) { return __builtin_fma(a, b, c); }

/*
 * Semantics pin, so the test also builds and runs under
 * scripts/run_regression_suite.sh (which links and A/B-compares every
 * every tests/regression C file against GCC) instead of assembly-only.
 *
 * The operands are chosen so fused and unfused evaluation differ: the exact
 * product carries bits below the destination precision, which a separate
 * multiply throws away before the add sees them.
 *   float : (1+2^-12)^2 - (1+2^-11) = 2^-24 fused, 0 unfused
 *   double: (1+2^-28)^2 - (1+2^-27) = 2^-56 fused, 0 unfused
 */
int main(void)
{
    const float a = 1.0f + 0x1p-12f;
    const float zf = -(1.0f + 0x1p-11f);
    const double b = 1.0 + 0x1p-28;
    const double zd = -(1.0 + 0x1p-27);
    long fail = 0;

    if (fused(a, a, zf) != 0x1p-24f)
        fail++;
    if (fused_d(b, b, zd) != 0x1p-56)
        fail++;
    /* The fold must not change ordinary values either. */
    if (fused(2.0f, 3.0f, 4.0f) != 10.0f)
        fail++;
    if (fused_d(2.0, 3.0, 4.0) != 10.0)
        fail++;

    printf("fail=%ld\n", fail);
    return fail != 0;
}
