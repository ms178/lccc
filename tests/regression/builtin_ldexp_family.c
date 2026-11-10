/* The `__builtin_ldexp`/`ldexpf`/`scalbn`/`scalbnf`/`frexp`/`frexpf` family.
 *
 * These were registered as libc aliases for the `l` (long double) spellings
 * only.  The `d` and `f` spellings were absent from BUILTIN_MAP, so sema
 * reported an implicit declaration; and even after registering them they
 * stayed untyped in `gcc_compat_builtin_ctype`, which made `builtin_return_ctype`
 * fall through to the implicit-`int` return.  The result was silently wrong,
 * not a diagnostic:
 *
 *     float f1(float f, int e) { return __builtin_ldexpf(f, e); }
 *     f_b(1.5f, 3) -> 1.09471e+09        (want 12)
 *
 * 1.09471e+09 is 0x4E828000, i.e. the bits of the returned 1.0f in xmm0 read as
 * an `int` and converted back to float -- the classic missing-return-type
 * signature.  `__builtin_ldexp` happened to survive because its `double`
 * return and the `int` path both travel in xmm0/rax without a conversion.
 *
 * The differential below compares every builtin against the plain libc call,
 * which is the definition GCC uses, over a range of exponents that crosses
 * subnormal, normal and overflow boundaries.
 */
#include <math.h>
#include <stdio.h>

static int bad;

static void check_int(const char *what, int got, int want) {
    if (got != want) { printf("  %s: got %d want %d\n", what, got, want); bad++; }
}
static void check_d(const char *what, double got, double want) {
    if (got != want) { printf("  %s: got %g want %g\n", what, got, want); bad++; }
}
static void check_f(const char *what, float got, float want) {
    if (got != want) { printf("  %s: got %g want %g\n", what, got, want); bad++; }
}

int main(void) {
    for (int e = -60; e <= 60; e += 5) {
        volatile double d = 1.5;
        volatile float  f = 1.5f;
        volatile double dn = -0.375;
        volatile float  fn = -0.375f;

        check_d("ldexp",  __builtin_ldexp(d, e),   ldexp(d, e));
        check_f("ldexpf", __builtin_ldexpf(f, e),  ldexpf(f, e));
        check_d("scalbn", __builtin_scalbn(d, e),  scalbn(d, e));
        check_f("scalbnf",__builtin_scalbnf(f, e), scalbnf(f, e));
        check_d("ldexp-neg",  __builtin_ldexp(dn, e),  ldexp(dn, e));
        check_f("ldexpf-neg", __builtin_ldexpf(fn, e), ldexpf(fn, e));

        /* frexp writes through a pointer, so its OUTPUT PARAMETER is part of
         * the contract -- a wrong return type hides there too. */
        int eb = 0, el = 0;
        float  fb = __builtin_frexpf(f, &eb),  fl = frexpf(f, &el);
        check_f("frexpf", fb, fl);
        check_int("frexpf-exp", eb, el);
        double db = __builtin_frexp(d, &eb), dl = frexp(d, &el);
        check_d("frexp", db, dl);
        check_int("frexp-exp", eb, el);
    }

    /* Literal exponents too: no aliasing to hide behind. */
    check_f("ldexpf(1.5f,3)", __builtin_ldexpf(1.5f, 3), 12.0f);
    check_d("ldexp(1.5,3)",   __builtin_ldexp(1.5, 3),   12.0);
    check_f("scalbnf(.375,4)",__builtin_scalbnf(0.375f, 4), 6.0f);

    if (bad) { printf("FAIL: %d builtin/libc mismatches\n", bad); return 1; }
    printf("PASS: ldexp/ldexpf/scalbn/scalbnf/frexp/frexpf match libc\n");
    return 0;
}
