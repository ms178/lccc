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
 * The four signed families (the operand-negation spellings the IR peel
 * folds into the family flags). Pinned at the AVX1-class target
 * (-mno-avx2): AVX2 is the 256-bit integer class, a strict subset of the
 * VEX encoding, so denying it must remove ymm code but keep every VEX.128
 * family — vfnmadd/vfmsub/vfnmsub included, exactly like GCC's
 * -march=x86-64-v3 -mno-avx2. The pre-fix ISA ceiling killed `avx` on
 * this flag, and these spelled xorpd + xorpd + a libm call.
 */
double signed_np(double a, double b, double c) { return __builtin_fma(-a, b, c); }
double signed_na(double a, double b, double c) { return __builtin_fma(a, b, -c); }
double signed_np_na(double a, double b, double c) { return __builtin_fma(-a, b, -c); }

/*
 * The chain shapes (the absorbability-fixpoint correction, now composed
 * with the neg-of-neg fold): a Neg may only die when every reader of it
 * dies with it -- and neg(neg(x)) is x (a bitwise identity), so a double
 * link folds away before the peel ever runs. `chain_live` is the shape the
 * source-poisoned rule miscompiled: the fma reads the INNER negation while
 * the OUTER survives for the add. The fold rewrites the outer link to x,
 * the inner's only remaining reader is the fma site, and the peel absorbs
 * it: ZERO sign masks, one vfnmadd reading x -- GCC's exact family. (The
 * peel-level rule -- a chain that DOES reach it with a surviving outer
 * reader keeps both links -- stays pinned by the unit test
 * chained_negation_with_surviving_outer_reader_is_never_deleted.)
 * `chain_pin`: the INNER negation is pinned by the add, the site reads the
 * OUTER -- which the fold rewrites to x directly, so the fma takes the
 * PLAIN family: ONE sign mask (the pinned inner) and one vfmadd. GCC drops
 * the mask too (r + (-x) -> r - x); that float add-of-neg fold is a
 * designed follow-up. Both live BEFORE shared_neg so the per-function sed
 * ranges below stay disjoint.
 */
double chain_live(double x, double b, double c)
{
    double t = -x;
    double u = -t;
    double r = __builtin_fma(t, b, c);
    return r + u;
}
double chain_pin(double x, double b, double c)
{
    double t = -x;
    double r = __builtin_fma(-t, b, c);
    return r + t;
}

/*
 * The shared-negation phi shape (one -b/-c read by TWO fma sites): the
 * multi-use absorbability rule must peel both sites at the AVX1-class
 * target too — two vfnmsub, no surviving xorpd bracket.
 */
double shared_neg(double a, double b, double c) {
    double hi = __builtin_fma(1.5, -b, -c);
    double lo = __builtin_fma(a, -b, -c);
    return hi * 3.0 + lo;
}

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
