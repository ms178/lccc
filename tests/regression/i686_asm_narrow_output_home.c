/* i686: a narrow inline-asm output promoted to an SSA value must leave its
 * 4-byte home fully defined; one in user memory must stay exact-width.
 *
 * `if (r)` on a `char` output simplifies to a CondBranch on the I8 value,
 * which i686 tests as `cmpl $0, home`.  The output was stored with
 * `movb %cl, home`, so the test read three stale bytes (with a poisoned
 * stack, sign_nonneg(-5) took the wrong branch at -O1/-O2).  Outputs into a
 * value home now store the type's extension (`movsbl/movzbl/movswl/movzwl`
 * + `movl`), like x86-64; outputs into a C object (array element, struct
 * field) keep `movb`/`movw`, so their neighbours survive.  */
#include <stdio.h>

__attribute__((noinline)) static void poison(void) {
    volatile unsigned char junk[512];
    for (int i = 0; i < 512; i++) junk[i] = 0xff;
}

__attribute__((noinline)) static int sign_nonneg(int x, int y) {
    char r;
    __asm__("testl %1, %1\n\tsetns %0" : "=q"(r) : "r"(x));
    int k = y * 3 + x;
    __asm__ volatile("" ::: "ebx", "esi", "edi", "memory");
    if (r) return k + 1;
    return k - 1;
}

__attribute__((noinline)) static int low_word_zero(unsigned x) {
    unsigned short w;
    __asm__("movw %w1, %w0" : "=r"(w) : "r"(x));
    __asm__ volatile("" ::: "ebx", "esi", "edi", "memory");
    return w ? 1 : 0;
}

__attribute__((noinline)) static int cc_out(int a, int b) {
    _Bool lt;
    __asm__("cmpl %2, %1" : "=@ccl"(lt) : "r"(a), "r"(b));
    __asm__ volatile("" ::: "ebx", "esi", "edi", "memory");
    return lt ? 30 : 40;
}

struct S { char a, b, c, d; short s, t; };

__attribute__((noinline)) static void mem_outs(struct S *p, char *buf) {
    __asm__("movb $0x11, %0" : "=q"(p->b));
    __asm__("movw $0x2222, %0" : "=r"(p->s));
    __asm__("movb $0x33, %0" : "=q"(buf[1]));
}

int main(void) {
    int r[6];
    poison(); r[0] = sign_nonneg(-5, 2);
    poison(); r[1] = sign_nonneg(5, 2);
    poison(); r[2] = low_word_zero(0x10000u);
    poison(); r[3] = low_word_zero(0x10001u);
    poison(); r[4] = cc_out(1, 2);
    poison(); r[5] = cc_out(2, 1);
    struct S s = {1, 2, 3, 4, 5, 6};
    char buf[4] = {7, 8, 9, 10};
    mem_outs(&s, buf);
    printf("%d %d %d %d %d %d | %d %d %d %d %d %d | %d %d %d %d\n", r[0], r[1], r[2], r[3],
           r[4], r[5], s.a, s.b, s.c, s.d, s.s, s.t, buf[0], buf[1], buf[2], buf[3]);
    return !(r[0] == 0 && r[1] == 12 && r[2] == 0 && r[3] == 1 && r[4] == 30 && r[5] == 40 &&
             s.a == 1 && s.b == 0x11 && s.c == 3 && s.d == 4 && s.s == 0x2222 && s.t == 6 &&
             buf[0] == 7 && buf[1] == 0x33 && buf[2] == 9 && buf[3] == 10);
}
