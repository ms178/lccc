/* Volatile 64-bit float/double constant stores must remain a SINGLE store
 * access.  PR #363's immediate-form float-constant lowering split an
 * 8-byte F64/D64 constant that does not fit a sign-extended imm32 into two
 * 4-byte `movl` halves.  A C `volatile` store is one abstract access
 * (C11 5.1.2.3): a signal handler or a concurrent reader may legally
 * observe the object between two halves, so the split is forbidden there
 * (the mature pool+movsd form is used instead, as gcc/clang/icc do).
 *
 * This oracle pins the runtime results of the split-susceptible shapes
 * (volatile double/float locals and through-pointer stores of constants
 * whose bit patterns do not fit imm32, plus 0.0 / -0.0 / fitting values
 * which the typed path may still emit as single immediate stores).  The
 * single-access property itself is pinned by the isel unit tests
 * (volatile_wide_f64_const_store_is_refused_not_split & friends); the
 * suite would not otherwise observe the two-store form, so both gates are
 * needed.
 */
#include <stdio.h>

volatile double gd;
volatile float gf;
volatile double g_neg0, g_zero;

static double wide1 = 1.5e300;   /* hi half 0x7E5E...  sign bit CLEAR */
static float  fwide = -1.75e30f; /* F32 pattern 0xF8...  (sign bit set)  */

int main(void) {
    volatile double t;
    volatile float f;
    /* wide double const into a volatile local: single-store required */
    t = 1.5e300;
    gd = t;
    /* wide float const (sign bit set in the imm32 window) */
    f = -1.75e30f;
    gf = f;
    /* exact-zero and negative-zero bit patterns */
    g_zero = 0.0;
    g_neg0 = -0.0;
    /* through-pointer volatile store of a wide double */
    { volatile double *p = &gd; *p = wide1; }
    printf("%d %d %d %d %d\n",
           gd == 1.5e300, gf == fwide, g_zero == 0.0,
           g_neg0 != g_zero /* -0.0 != +0.0 under == */,
           gd == wide1);
    return 0;
}
