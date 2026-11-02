/* .symver on an alias-defined symbol + intra-TU reference (glibc libm
 * e_j0f.c shape, IS-29 follow-up: `undefined reference to __j0f_finite`).
 *
 * glibc's libm_alias_finite expands to
 *     strong_alias (__j0f, __ieee754_j0f)              // alias DEFINITION
 *     __asm__(".symver __ieee754_j0f, __j0f_finite@GLIBC_2.15");
 * and y0f() calls __ieee754_j0f() in the same TU.
 *
 * GAS semantics: `.symver real, name@VER` where `real` is DEFINED in the
 * object creates the versioned alias for export; intra-object references to
 * `real` stay bound to the local definition. Only when `real` is UNDEFINED
 * do references get rewritten to the versioned name.
 *
 * Fixed defect: the lowering pre-pass classified __attribute__((alias))
 * declarators as "not defined" (they parse as extern declarations without
 * initializers), applied the undefined-reference flavour, and rewrote the
 * call into a dangling versioned reference — every libm.so link failed.
 *
 * This test is a link-or-die reproducer: before the fix the executable
 * cannot link (undefined reference to exported_compat).
 */
int
impl_fn (int x)
{
  return x * 3;
}
extern __typeof (impl_fn) alias_fn __attribute__ ((alias ("impl_fn")));
__asm__ (".symver alias_fn, exported_compat@COMPAT_1");

int
caller (int x)
{
  /* Must bind to the local definition, NOT become exported_compat@COMPAT_1. */
  return alias_fn (x);
}

int
main (void)
{
  int ok = caller (14) == 42;
  __builtin_printf ("symver-alias-defined:%s\n", ok ? "ok" : "MISMATCH");
  return ok ? 0 : 1;
}
