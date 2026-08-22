/* Homeless-value materialization: rematerialisable GlobalAddr in generic
 * value-load paths, and absolute-address dereferences.
 *
 * Two silent-zero fabrication sites (same disease class as the
 * operand_to_rax session-26 hard gate) fixed after Agent C's GCC-torture
 * triage (execute/20000412-6.c, compile/HIcmp.c family):
 *
 *  1. operand_to_rcx's last-resort arm emitted `xorl %ecx,%ecx` for a value
 *     with no home — a rematerialisable GlobalAddr base of `&buf[k]` became
 *     0 and the pointer arithmetic produced a small integer instead of an
 *     address.
 *  2. value_to_reg panicked (or, in older builds, read a stale slot) for
 *     the same shape and for literal absolute-address loads
 *     (`*(short*)0x11111111` read a stack slot instead of the address).
 *
 * Both paths now rebuild the GlobalAddr address (GOT/TLS/absolute-aware)
 * or materialize the constant address into the register.
 */
#include <stdio.h>

static char buf[16] = "0123456789abcdef";

/* &buf[k] with a runtime index: the GlobalAddr base is rematerialisable
 * and must be rebuilt, never fabricated as zero. */
__attribute__((noinline)) char *pick(int k) { return &buf[k]; }

/* Compile-only: literal absolute addresses must produce a real
 * materialize+dereference sequence (checked by executing pick/ptr math,
 * and by this TU merely compiling without ICE at -O2). */
short absload(short r0, long addr)
{
    if (addr == 0) /* never true at runtime with our argument */
        return (short)(r0 <= *(short *)0x11111111);
    return r0;
}

int main(void)
{
    char *p = pick(3);
    long off = p - buf;
    int ok = (*p == '3') && (off == 3);
    ok &= pick(0) == buf;
    ok &= *pick(15) == 'f';
    ok &= absload(7, 1) == 7;
    printf("remat:%s off=%ld\n", ok ? "ok" : "MISMATCH", off);
    return ok ? 0 : 1;
}
