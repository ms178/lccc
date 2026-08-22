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

int main(void) {
    unsigned long canary = direct_cast();
    int ok = canary != 0;
    ok &= qualifier_after_base() == canary;
    ok &= via_variable() == canary;
    ok &= via_typedef_cast() == canary;
    ok &= via_typedef_variable() == canary;
    ok &= phi_const_ptr(sel) == canary;
    ok &= phi_const_ptr(0) != 0; /* %fs:0 = TCB self pointer */
    ok &= no_leak() == 7;
    /* Print a stable token (not the canary value itself) for the
     * cross-compiler stdout comparison. */
    printf("segfs-matrix:%s\n", ok ? "ok" : "MISMATCH");
    return ok ? 0 : 1;
}
