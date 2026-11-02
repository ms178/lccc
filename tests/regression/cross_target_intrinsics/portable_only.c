/* AUD-SILENT-ZERO-1 companion fixture: PORTABLE builtins only.
 *
 * The gate next to this file (check_cross_target_intrinsic_trap.sh) compiles
 * this translation unit for all four targets and the x86-only one for the x86
 * targets only.  Splitting them is what makes the negative test possible: a
 * single TU mixing both would be rejected as a whole off x86, and "rejected"
 * would then also hide a regression that rejects the portable builtins --
 * `sqrt`/`fabs`/`popcount` must keep lowering on every backend.
 */
double portable_sqrt(double x) { return __builtin_sqrt(x); }

double portable_fabs(double x) { return __builtin_fabs(x); }

int portable_popcount(unsigned int x) { return __builtin_popcount(x); }
