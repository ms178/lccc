/* Conditional-sum guard (if_convert Select) must survive vectorization.
 *
 * `if (a[i] > 0) s += a[i]` lowers to `s' = Select(a[i] > 0, s + a[i], s)`.
 * The old equal-width vector transform replaced the scalar Add with a plain
 * vector add and REMOVED the Select — silently summing every element:
 * lccc printed 612 for the 17-element kernel below where GCC prints 651
 * (612 = 7*136 - 20*17 = the UNGUARDED sum; regression introduced by the
 * "masked widening" work which only guarded the widening form).  The fix
 * emits the equal-width masked step (x86 vpcmpgtd/vpand/vpaddd) whose
 * per-lane mask reproduces `lane > rhs` exactly, or keeps the loop scalar
 * for non-I32 elements.
 *
 * The swapped form (`if (a[i] & 1) continue; s += a[i]`, i.e.
 * Select(cond, s, s + x)) is rejected up front: the masked intrinsic only
 * encodes add-when-cond-true.
 */
#include <stdio.h>
volatile int vn17 = 17, vn9 = 9, vn5 = 5;
int A[64], B[64];
static void init(void) {
    for (int i = 0; i < 64; i++) { A[i] = i * 7 - 20; B[i] = (i * 13) % 11 - 5; }
}
#define NOINLINE __attribute__((noinline))
/* Natural form: guard on the TRUE arm. */
NOINLINE int k_guard_nat(int n) {
    int s = 0;
    for (int i = 0; i < n; i++) { if (A[i] > 0) s += A[i]; }
    return s;
}
/* Rhs is a runtime parameter. */
NOINLINE int k_guard_rhs_rt(int n, int lim) {
    int s = 0;
    for (int i = 0; i < n; i++) { if (A[i] > lim) s += A[i]; }
    return s;
}
/* Guarded widening (long accumulator): the masked widening path. */
NOINLINE long k_guard_widen(int n) {
    long s = 0;
    for (int i = 0; i < n; i++) { if (A[i] > 0) s += A[i]; }
    return s;
}
/* Swapped form: add on the FALSE arm (continue-skip idiom). */
NOINLINE int k_guard_swapped(int n) {
    int s = 0;
    for (int i = 0; i < n; i++) { if (A[i] & 1) continue; s += A[i]; }
    return s;
}
/* Guarded FP sum: must stay scalar (correct), never lose the guard. */
NOINLINE double k_guard_fp(int n) {
    double s = 0;
    for (int i = 0; i < n; i++) { if (A[i] > 0) s += A[i] * 0.5; }
    return s;
}
int main(void) {
    init();
    printf("nat=%d %d\n", k_guard_nat(vn17), k_guard_nat(vn9));
    printf("rhs_rt=%d %d\n", k_guard_rhs_rt(vn17, 0), k_guard_rhs_rt(vn17, -3));
    printf("widen=%ld\n", k_guard_widen(vn17));
    printf("swapped=%d\n", k_guard_swapped(vn17));
    printf("fp=%.1f\n", k_guard_fp(vn5));
    return 0;
}
