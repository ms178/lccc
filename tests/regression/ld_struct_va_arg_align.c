/* struct { long double } through va_arg: caller/callee alignment agreement.
 *
 * The RISC-V LP64D / AAPCS64 / SysV rules all align stack arguments whose
 * natural alignment exceeds XLEN to min(align, 2*XLEN) in the argument
 * area, and va_arg applies the same rounding before reading.  The shared
 * classification previously rounded struct stack slots to 8 bytes only,
 * so a struct { long double } (align 16) placed after an odd 8-byte slot
 * was read from a different offset than the caller wrote it — va_arg
 * returned garbage (current_tasks/fix_riscv_va_arg_long_double_struct,
 * test_misalign_r10 shape).
 *
 * This host test locks the shared compute_stack_arg_padding logic on
 * x86-64 with the misalignment shape: an odd 8-byte variadic argument
 * precedes each struct, so the struct must land on a 16-byte boundary.
 * The same padding code drives the RISC-V and AArch64 callers.
 */
#include <stdarg.h>

struct LD {
    long double x;
};

/* Interleave scalars and over-aligned structs so several padding
 * decisions must compose: 1 slot, struct, 2 slots, struct. */
static long double consume(int n, ...)
{
    va_list ap;
    va_start(ap, n);
    int pad1 = va_arg(ap, int);
    struct LD a = va_arg(ap, struct LD);
    int p1 = va_arg(ap, int);
    int p2 = va_arg(ap, int);
    struct LD b = va_arg(ap, struct LD);
    va_end(ap);
    if (pad1 != 7 || p1 != 11 || p2 != 13)
        return -1.0L;
    return a.x * 4.0L + b.x + (long double)n;
}

int main(void)
{
    struct LD a, b;
    a.x = 1.5L;
    b.x = 2.25L;
    /* 1.5*4 + 2.25 + 10 = 18.25 */
    long double r = consume(10, 7, a, 11, 13, b);
    if ((double)r != 18.25)
        return 1;
    return 0;
}
