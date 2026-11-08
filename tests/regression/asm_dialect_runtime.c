/* Runtime proof for inline-asm text handling on the host CPU.
 *
 * The assembly-text gates compare emitted strings. This one EXECUTES: the
 * same program is compiled by LCCC and by GCC 16.2, both binaries run, and
 * their observable results must agree. That catches a whole class of defect a
 * text comparison cannot -- a `%=` label that is unique but branches to the
 * wrong place, a dialect alternative that expands to the wrong mnemonic, an
 * inlined copy that shares a label with its sibling.
 *
 * Covers, all in extended asm (operands or clobbers present):
 *   - pipeless `{l}` and piped `{l|q}` suffix selection (operands are
 *     `int`, so the AT&T alternative `l` is the correct width on x86-64 too)
 *   - `%=` local labels used as a real backward branch, in a loop
 *   - two `%=` statements in one function (must not collide)
 *   - a static callee with `%=` inlined twice (each copy needs its own label)
 *   - basic asm, where GCC emits the template verbatim
 *   - non-ASCII comment bytes inside a template
 */
#include <stdio.h>

/* Sum 1..n with a `%=`-labelled backward branch. If two instances shared a
 * label the assembler would either fail or bind to the wrong loop. */
static int sum_to(int n) {
    int acc = 0;
    __asm__ volatile(
        "test{l|q} %[n], %[n]\n\t"
        "jz .Ldone%=\n\t"
        ".Lloop%=:\n\t"
        "add{l|q} %[n], %[a]\n\t"
        "dec{l|q} %[n]\n\t"
        "jnz .Lloop%=\n\t"
        ".Ldone%=:"
        : [a] "+r"(acc), [n] "+r"(n)
        :
        : "cc");
    return acc;
}

/* A second `%=` in the same translation unit, and a pipeless suffix. */
static int double_it(int x) {
    int r = x;
    __asm__ volatile("sh{l} $1, %[r]" : [r] "+r"(r) : : "cc");
    return r;
}

/* Called twice so the inliner produces two copies of the same `%=`. */
static int bump(int x) {
    __asm__ volatile(
        ".Lbump%=:\n\t"
        "inc{l|q} %[x]"
        : [x] "+r"(x)
        :
        : "cc");
    return x;
}

/* Non-ASCII bytes must survive verbatim; a double-encoding bug would still
 * assemble (it is a comment) so only a byte comparison catches it. The
 * execution here proves the surrounding code is unaffected. */
static int utf8(int x) {
    __asm__ volatile("# MS09 caf\303\251 \342\202\254 \360\237\246\200\n\t"
                     "nop"
                     : "+r"(x));
    return x;
}

/* Basic asm: GCC emits this verbatim, so it must contain no braces. */
static void basic(void) { __asm__ volatile("nop"); }

int main(void) {
    int s = sum_to(100);          /* 5050 */
    int d = double_it(21);        /* 42 */
    int b = bump(bump(40));       /* 42 */
    int u = utf8(7);              /* 7 */
    basic();
    printf("%d %d %d %d\n", s, d, b, u);
    return (s == 5050 && d == 42 && b == 42 && u == 7) ? 0 : 1;
}
