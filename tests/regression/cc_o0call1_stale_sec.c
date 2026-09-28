/* CC-O0CALL-1: stale secondary-cache residency across call-argument staging.
 *
 * At -O0 the arg1 comparison parks &u in the secondary (%rcx) cache while
 * computing the boolean.  Staging the scalar 4th argument (`*p`, SysV GP
 * arg4) then overwrites %rcx; the trailing 16-byte by-value union consults
 * the stale SEC entry and dereferences the clobbered register instead of
 * reloading the address from its slot.
 *
 * Pre-fix (6f8ace9c) -O0 shape, proven by differential assembly: the byval
 * first-eightbyte load reads through the dead register (`movq (%rax), %r8`
 * with %rax == 0, the -O0 peephole having folded the `movq %rcx, %rax` SEC
 * consume into the preceding `movq %rax, %rcx` staging move) -> SIGSEGV.
 * Post-fix the union reloads from its slot (`movq -8(%rbp), %r8`).
 *
 * Distinct from the 8 upstream call_secondary_cache_clobber.c shapes, whose
 * trailing argument is always the 8-byte union U (single %r8 eightbyte):
 * this pins the 16-byte INTEGER-class byval (%r8 + stack split) after a
 * scalar %rcx clobber.
 *
 * Fix: the per-argument previous_arg_wrote_rcx deferred invalidate in
 * src/backend/x86/codegen/calls.rs (Phase 3), mechanically enforced by the
 * %rcx shadow-epoch validator (CodegenState::sec_has_verified).  The
 * Phase-2 dynamic-realignment scratch is covered separately (F-CC11), and
 * emit_acc_to_secondary_impl maintains the same contract on its %rcx write.
 */
#include <stdio.h>

typedef union {
    long a;
    long b;
} U1;

static int g;

static int f(int a0, int a1, int a2, int a3, U1 a4)
{
    return a0 + a1 + a2 + a3 + (int)a4.a;
}

int main(void)
{
    U1 u;
    int *p = &g;
    u.a = 0;
    int got = f(1, (&u == &u), 3, *p, u);
    int expected = 1 + 1 + 3 + 0 + 0;
    printf("%d %d\n", got, expected);
    return got != expected;
}
