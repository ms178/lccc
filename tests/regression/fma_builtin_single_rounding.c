/* C99 fma()/fmaf() single-rounding semantics via __builtin_fma{,f}.
 *
 * glibc 2.44's s_fma.c/s_fmaf.c (built with the ms178 patch's
 * math-use-builtins-fma.h) reduce to `return __builtin_fma(x, y, z);` —
 * without a real fused lowering the TU emits an undefined call and libm.so
 * cannot link. Splitting into Mul+Add is NOT an acceptable lowering: fma
 * requires a SINGLE rounding (C99 F.10.10.1).
 *
 * The separation vector (1+2^-52)(1-2^-52) - 1:
 *   exact       = -2^-104            (representable!)
 *   fused       -> -0x1p-104
 *   double-rounded (mul rounds to 1.0) -> 0.0
 * Any implementation that contracts through a rounded product fails this.
 *
 * The emitted vfmadd231sd/ss encodings are pinned against GNU as by
 * tests/asm-diff/fma-scalar.casefile (insndiff/asmdiff oracles).
 */
#include <stdio.h>

__attribute__((noinline)) double via_fma(double a, double b, double c)
{
    return __builtin_fma(a, b, c);
}

__attribute__((noinline)) float via_fmaf(float a, float b, float c)
{
    return __builtin_fmaf(a, b, c);
}

__attribute__((noinline)) double via_muladd(double a, double b, double c)
{
    volatile double p = a * b; /* force the double rounding */
    return p + c;
}

int main(void)
{
    double f = via_fma(1.0 + 0x1p-52, 1.0 - 0x1p-52, -1.0);
    double m = via_muladd(1.0 + 0x1p-52, 1.0 - 0x1p-52, -1.0);
    float ff = via_fmaf(1.0f + 0x1p-23f, 1.0f - 0x1p-23f, -1.0f);

    int ok = (f == -0x1p-104) && (m == 0.0) && (f != m);
    ok &= (ff == -0x1p-46f);
    /* plain values too */
    ok &= via_fma(2.0, 3.0, 4.0) == 10.0;
    ok &= via_fmaf(2.0f, 3.0f, 4.0f) == 10.0f;
    /* negative-zero sign: fma(0,0,-0) = -0? No: +0 + -0 = +0; but
       fma(-0.0, 5.0, -0.0) = -0.0 exactly. */
    ok &= __builtin_signbit(via_fma(-0.0, 5.0, -0.0)) != 0;
    printf("fma=%a muladd=%a fmaf=%a %s\n", f, m, (double)ff,
           ok ? "ok" : "MISMATCH");
    return ok ? 0 : 1;
}
