/* GCC named address spaces (__seg_fs) — full declarator/typedef/flow matrix.
 *
 * Fixed defects (glibc 2.44 csu/libc-start.c, __libc_start_main_impl):
 *
 *  1. post-phi copy cleanup removed `v = Copy(Const 16)` feeding
 *     `Load %fs:(v)`: Value-position fields (Load.ptr, Store.ptr, GEP.base)
 *     cannot hold a constant, `replace_value_in_place` silently skipped the
 *     rewrite, and the load's only def vanished (backend no-home ICE).
 *
 *  2. `build_full_ctype*` hardcoded AddressSpace::Default on every derived
 *     Pointer, so NAMED variables and typedefs of segment pointers lost the
 *     qualifier — their dereferences emitted absolute-address loads
 *     (SIGSEGV on the NULL page). Only direct `*(T __seg_fs *)N` casts
 *     survived.
 *
 *  3. Initializer casts to segment-qualified pointers `mem::take` the
 *     parser's parsing_address_space, clearing the declaration-level
 *     qualifier before Declaration::new read it
 *     (`T __seg_fs *p = (T __seg_fs *)40;` lost %fs entirely).
 *
 * %fs:40 is the x86-64 TLS stack canary and %fs:16 the TCB self pointer —
 * both always mapped and readable in a live glibc process, which makes this
 * miscompile class *executable*: a regressing compiler segfaults on the
 * NULL-page load instead of printing matching values.
 */
#include <stdio.h>

typedef unsigned long __seg_fs *fsp_t;

static unsigned long direct_cast(void) {
    return *(__seg_fs unsigned long *)40;
}

static unsigned long qualifier_after_base(void) {
    return *(unsigned long __seg_fs *)40;
}

static unsigned long via_variable(void) {
    unsigned long __seg_fs *p = (unsigned long __seg_fs *)40;
    return *p;
}

static unsigned long via_typedef_cast(void) { return *(fsp_t)40; }

static unsigned long via_typedef_variable(void) {
    fsp_t p = (fsp_t)40;
    return *p;
}

/* Qualifier BEFORE the base type at declaration level (`__seg_fs T *p`),
 * with the pending qualifier flowing spec-qual-list -> declaration
 * snapshot -> apply_declaration_address_space onto the pointer. The
 * initializer cast re-uses the before-base spelling, exercising the
 * nested-type-name scoping from the other side: the cast's own
 * qualifier must reach the cast's `*` even though a (possibly live)
 * enclosing pending slot is saved/restored around it.
 */
static unsigned long qualifier_before_base_decl(void) {
    __seg_fs unsigned long *p = (__seg_fs unsigned long *)40;
    return *p;
}

/* DECLARE_PER_CPU(T, name) expands to `__seg_fs __typeof__(T) name`:
 * the qualifier sits before the __typeof__ and must land on the
 * DECLARED object (via the declaration snapshot), whatever T is.
 * T = unsigned long here: the typeof argument has no `*`, so the
 * pending qualifier survives even an unscoped parse; the runnable
 * value proves the snapshot -> pointer-AS application still works.
 */
static unsigned long typeof_qual_before_base(void) {
    __seg_fs __typeof__(unsigned long) *p = (__seg_fs unsigned long *)40;
    return *p;
}

/* The typeof argument's OWN internal qualifier (after its base, before
 * its `*`) must reach that inner `*` under nested-type-name scoping:
 * the scoping clears the pending slot at entry, the argument's
 * `__seg_fs` re-fills it, its `*` consumes it.
 */
static unsigned long typeof_internal_qualifier(void) {
    __typeof__(unsigned long __seg_fs *) p = (unsigned long __seg_fs *)40;
    return *p;
}

/* A cast whose type-name has an internal array-of-pointer declarator:
 * the suffix parser must take the qualifier the cast's spec-qual-list
 * set before its own base type, through the nested-type-name scoping.
 */
static unsigned long cast_array_of_fs_pointers(void) {
    unsigned long __seg_fs *arr[1];
    arr[0] = (__seg_fs unsigned long *)40;
    return *arr[0];
}

/* Pointer selected between two constant TLS offsets: lowers to
 * `phi-home = Copy(Const)` feeding a SegFs load — the exact
 * __libc_start_main_impl shape that ICEd. %fs:0 holds the TCB self
 * pointer (nonzero), %fs:40 the canary. */
static volatile int sel = 1;
static unsigned long phi_const_ptr(int c) {
    fsp_t p = (fsp_t)(unsigned long)(c ? 40 : 0);
    return *p;
}

/* The qualifier must not leak into following declarations. */
static unsigned long plain_val = 7;
static unsigned long no_leak(void) {
    unsigned long *q = &plain_val; /* ordinary pointer */
    return *q;
}

/* Forward declaration: segstore_conflation is defined below main (next to
 * its long rationale comment); declaring it here keeps the GCC oracle in
 * the comparison — GCC 14's default gnu23 mode treats the implicit
 * declaration as an error, which silently downgraded this test to
 * lccc-only (SKIP-COMPARE) instead of a differential run. */
__attribute__((noinline)) static unsigned long segstore_conflation(void);

int main(void) {
    unsigned long canary = direct_cast();
    int ok = canary != 0;
    ok &= qualifier_after_base() == canary;
    ok &= via_variable() == canary;
    ok &= via_typedef_cast() == canary;
    ok &= via_typedef_variable() == canary;
    ok &= qualifier_before_base_decl() == canary;
    ok &= typeof_qual_before_base() == canary;
    ok &= typeof_internal_qualifier() == canary;
    ok &= cast_array_of_fs_pointers() == canary;
    ok &= phi_const_ptr(sel) == canary;
    ok &= phi_const_ptr(0) != 0; /* %fs:0 = TCB self pointer */
    ok &= no_leak() == 7;
    ok &= segstore_conflation() == 1;
    /* Print a stable token (not the canary value itself) for the
     * cross-compiler stdout comparison. */
    printf("segfs-matrix:%s\n", ok ? "ok" : "MISMATCH");
    return ok ? 0 : 1;
}

/* Regression (LK-24 stage 3): SegFs STORE operand conflation. The fs-offset
 * constant's register home (%rdx) was clobbered by the stored VALUE before
 * the pointer was read, so THREAD_SETMEM-shaped code stored through the
 * VALUE as the address. Shape: value is an address derived from a SegFs
 * load, pointer is a small constant with a register home. %fs:16 is the
 * TCB self pointer: fs:16+0x30-ish offsets are inside our own (glibc)
 * pthread struct, so writing there is NOT safe in-process; instead verify
 * pure codegen shape via a round-trip through a real TLS variable. */
static __thread unsigned long tls_cell[4];
__attribute__((noinline)) static unsigned long segstore_conflation(void) {
    /* address-of-TLS derived value + constant-offset store target in one
     * expression forces both operands into registers simultaneously */
    unsigned long *p = &tls_cell[1];
    tls_cell[2] = (unsigned long)p; /* value IS an address */
    tls_cell[1] = 0x5150;
    return *(unsigned long *)tls_cell[2] == 0x5150;
}
