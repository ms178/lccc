/* Dense switches whose smallest case is NEGATIVE index their jump table with
 * `value - min`.  The x86-64 emitter sign-extends the 32-bit (or narrower)
 * scrutinee with `cltq` and rebases it with a 64-bit `subq $min, %rax`.
 *
 * The peephole pass `eliminate_dead_inplace_ext` deleted that `cltq` because
 * it treated the `subq` -- a read-modify-write -- as a pure redefinition of
 * %rax.  -1 then became 0x00000000ffffffff, rebased to 0x1_0000_0001, failed
 * the bounds check and dispatched to the DEFAULT arm
 * (gcc.c-torture/execute/20010106-1.c at -O0).
 *
 * Every row below is checked at the boundary values of its case range and on
 * both sides of it, for int, short and signed char scrutinees, and for a
 * range that straddles zero as well as one that is entirely negative.
 */
#include <stdio.h>

__attribute__((noinline)) static int f_int(int i) {
    switch (i) {
    case -2: return 33;
    case -1: return 0;
    case 0: return 7;
    case 1: return 4;
    case 2: return 3;
    case 3: return 15;
    case 4: return 9;
    default: return -100;
    }
}

__attribute__((noinline)) static int f_short(short i) {
    switch (i) {
    case -3: return 1;
    case -2: return 2;
    case -1: return 3;
    case 0: return 4;
    case 1: return 5;
    case 2: return 6;
    default: return -100;
    }
}

__attribute__((noinline)) static int f_schar(signed char i) {
    switch (i) {
    case -128: return 10;
    case -127: return 11;
    case -126: return 12;
    case -125: return 13;
    case -124: return 14;
    case -123: return 15;
    default: return -100;
    }
}

__attribute__((noinline)) static int f_allneg(int i) {
    switch (i) {
    case -20: return 1;
    case -19: return 2;
    case -18: return 3;
    case -17: return 4;
    case -16: return 5;
    case -15: return 6;
    case -14: return 7;
    default: return -100;
    }
}

int main(void) {
    static const int want_int[] = {-100, 33, 0, 7, 4, 3, 15, 9, -100};
    static const int want_short[] = {-100, 1, 2, 3, 4, 5, 6, -100};
    static const int want_schar[] = {10, 11, 12, 13, 14, 15, -100};
    static const int want_allneg[] = {-100, 1, 2, 3, 4, 5, 6, 7, -100};
    unsigned sum = 0;
    int bad = 0;
    for (int k = 0; k < 9; k++) {
        int r = f_int(k - 3);
        bad |= r != want_int[k];
        sum = sum * 31u + (unsigned)r;
    }
    for (int k = 0; k < 8; k++) {
        int r = f_short((short)(k - 4));
        bad |= r != want_short[k];
        sum = sum * 31u + (unsigned)r;
    }
    for (int k = 0; k < 7; k++) {
        int r = f_schar((signed char)(k - 128));
        bad |= r != want_schar[k];
        sum = sum * 31u + (unsigned)r;
    }
    for (int k = 0; k < 9; k++) {
        int r = f_allneg(k - 21);
        bad |= r != want_allneg[k];
        sum = sum * 31u + (unsigned)r;
    }
    /* Out-of-range extremes must reach the default arm, never the table. */
    bad |= f_int(-2147483647 - 1) != -100 || f_int(2147483647) != -100;
    bad |= f_allneg(-2147483647 - 1) != -100 || f_allneg(-13) != -100;
    bad |= f_short(-32768) != -100 || f_short(32767) != -100;
    bad |= f_schar(127) != -100;
    printf("sum=%u bad=%d\n", sum, bad);
    return bad;
}
