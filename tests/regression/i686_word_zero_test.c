/* i686: a 32-bit zero test of a zero-extended WORD must test 16 bits.
 *
 * fold_narrow_load_imm_compare rewrote `movzwl M, %r; testl %r, %r` as
 * `cmpb $0, M` (one byte: 0x0100 compared equal to zero) and, for a
 * register source, `cmpb $0, %ax` (which assemblers silently re-encode as
 * `cmpb $0, %al`).  Every truthiness test of an `unsigned short` whose low
 * byte is zero took the wrong branch at -O0, -O1 and -O2.  The fixed fold
 * emits `cmpw $0, M` / `testw %r, %r`.  i686_word_zero_test_o0.c runs the
 * same checks at -O0 (register-source shape).  */
#include <stdio.h>

struct S { unsigned short a, b; };

__attribute__((noinline)) static int nz(const unsigned short *p) { return *p != 0; }
__attribute__((noinline)) static int br(const unsigned short *p) { if (*p) return 7; return 9; }
__attribute__((noinline)) static int fld(struct S *s) { unsigned short x = s->b; return x == 0 ? 1 : 2; }
__attribute__((noinline)) static int sh(const short *p) { return *p ? 3 : 4; }
static volatile unsigned short g = 0x0100;
__attribute__((noinline)) static int glob(void) { unsigned short x = g; return x ? 5 : 6; }

int main(void) {
    unsigned short v = 0x0100, z = 0, ff = 0xff00;
    short neg = (short)0x8000;
    struct S s = {0x0100, 0x0200}, t = {1, 0};
    int r[] = {nz(&v), br(&v), fld(&s), glob(), nz(&ff), br(&z), fld(&t), sh(&neg)};
    int want[] = {1, 7, 2, 5, 1, 9, 1, 3};
    int bad = 0;
    for (unsigned i = 0; i < sizeof r / sizeof r[0]; i++) {
        printf("%d ", r[i]);
        bad |= r[i] != want[i];
    }
    printf("\n");
    return bad;
}
