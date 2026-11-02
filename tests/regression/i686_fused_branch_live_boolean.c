/* i686: compare-branch fusion must keep a boolean that is live past the jump.
 *
 * `switch ((x < 0) + 0)` lowers to `cmp; setl %al; movzbl; testl; je C0`
 * with the fallthrough testing the boolean again (`cmpl $1, %eax`).
 * fuse_compare_and_branch deleted the setl/movzbl along with the redundant
 * test, so the fallthrough compared whatever %eax held on entry: through a
 * function pointer (%eax = the callee address) sw_cmp(-1) returned 0, not
 * 11.  The fixed pass removes only the re-test unless liveness proves %eax
 * dead after the jump and no store published the boolean.  */
#include <stdio.h>

static __attribute__((noinline)) int sw_cmp(int x) {
    switch ((x < 0) + 0) { case 0: return 10; case 1: return 11; }
    return 0;
}
static __attribute__((noinline)) int sw_eq(unsigned x) {
    switch ((x == 7u) + 0) { case 0: return 20; case 1: return 21; }
    return 0;
}
int (*volatile fp)(int) = sw_cmp;
int (*volatile fq)(unsigned) = sw_eq;

int main(void) {
    int a = fp(-1), b = fp(5), c = sw_cmp(-3), d = fq(7), e = fq(8);
    printf("%d %d %d %d %d\n", a, b, c, d, e);
    return !(a == 11 && b == 10 && c == 11 && d == 21 && e == 20);
}
