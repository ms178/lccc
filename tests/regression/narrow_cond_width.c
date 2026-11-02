// Narrow-condition select/branch semantics across widths.
//
// Conditions in `?:` and `if` are tested against zero at the C width of the
// condition value. The x86 backend must size its in-place condition test to
// that width (testl/testw/testb), never testq on a value narrower than 64
// bits — the SysV x86-64 ABI leaves the bits above a parameter's width
// undefined, so a 64-bit test can read garbage that flips ZF.
//
// This is a pure-semantics differential (both compilers agree on outputs);
// the assembly-shape half of the regression lives in
// check_narrow_cond_width.sh, which additionally drives a dirty-caller that
// really passes undefined upper bits.
#include <stdio.h>

int s32(unsigned c, int a, int b) { return c ? a : b; }
int s32s(int c, int a, int b) { return c ? a : b; }
long s64(unsigned long c, long a, long b) { return c ? a : b; }
unsigned selc32(unsigned c) { return c ? 0x0badc0deu : 0x13579bdfu; }
short s16(short c, short a, short b) { return c ? a : b; }
int fbr(unsigned c) {
    int r = 0;
    if (c) r += 3;
    if (c > 3u) r += 5;
    return r;
}
long fbr64(unsigned long c) {
    long r = 0;
    if (c) r += 3;
    if (c > 3u) r += 5;
    return r;
}

int main(void) {
    unsigned vs[] = {0, 1, 2, 3, 4, 7, 0x7fffffff, 0x80000000u, 0xfffffffeu, 0xffffffffu};
    for (int i = 0; i < 10; i++) {
        unsigned c = vs[i];
        int r = s32(c, 111, -222);
        int rs = s32s((int)c, 111, -222);
        long rl = s64(c, 0x123456789LL, -0x13579bdfLL);
        unsigned rc = selc32(c);
        short r16 = s16((short)c, (short)5, (short)-9);
        int rb = fbr(c);
        long rb64 = fbr64(c);
        printf("%d %d %lld %u %d %d %lld\n", r, rs, rl, rc, (int)r16, rb, rb64);
    }
    return 0;
}
