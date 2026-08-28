/*
 * `!x` must test x at x's own (promoted) width for every integer type,
 * including sub-int types (which must be widened before the test — narrow
 * ops leave stale high bits in a register) and 64-bit types on ILP32
 * (a 32-bit test would truncate 0x1_0000_0000 to zero).  Regression for
 * the LogicalNot width fix in the IR lowering (expr_ops.rs).
 *
 * Values deliberately include 64-bit numbers whose low 32 bits are zero:
 * on a 32-bit target the old width-confused lowering reported `!x == 1`.
 */
#include <stdio.h>

#define NOINLINE __attribute__((noinline))

NOINLINE int not_uc(unsigned char x) { return !x; }
NOINLINE int not_us(unsigned short x) { return !x; }
NOINLINE int not_u(unsigned x) { return !x; }
NOINLINE int not_ull(unsigned long long x) { return !x; }
NOINLINE int not_ll(long long x) { return !x; }
NOINLINE int not_p(const void *p) { return !p; }

int main(void) {
    /* sub-int boundary values */
    int r = 0;
    r = r * 3 + not_uc(0);
    r = r * 3 + not_uc(1);
    r = r * 3 + not_uc(0x100 - 1);      /* all storage bits set */
    r = r * 3 + not_us(0);
    r = r * 3 + not_us(0x10000 - 1);
    /* 32-bit values (spill-sensitive: memory-operand cmp forms) */
    r = r * 3 + not_u(0);
    r = r * 3 + not_u(0x80000000u);
    r = r * 3 + not_u(0xffffffffu);
    /* 64-bit values with zero low halves */
    r = r * 3 + not_ull(0);
    r = r * 3 + not_ull(0x100000000ULL);   /* low 32 bits are zero */
    r = r * 3 + not_ull(0x1000000000000000ULL);
    r = r * 3 + not_ull(0xffffffff00000000ULL);
    r = r * 3 + not_ull(1);
    r = r * 3 + not_ll(-1);
    r = r * 3 + not_ll(0x100000000LL);
    /* pointers */
    r = r * 3 + not_p(0);
    r = r * 3 + not_p(&r);
    printf("%d\n", r);
    return 0;
}
