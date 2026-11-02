/* A structure-returning call must not be pointed at an object the callee can
 * reach through another pointer.
 *
 *     x = make(&x);
 *
 * lowers to `call make(%tmp, %x); memcpy %x <- %tmp`.  Retargeting the hidden
 * slot at %x and deleting the copy (sret_copy_elision phase 1, as merged in
 * #890) makes the callee's result writes land in the object it is still
 * reading.  The SysV ABI lets a callee write its return slot at any time,
 * because callers guarantee the slot aliases nothing else; GCC enforces that
 * with dest_safe_for_nrv_p (an address-taken destination is never used).
 *
 * The callee is assembly so the test pins the caller's contract only: it
 * writes slot.v[0] FIRST and then re-reads p->v[0], as an NRV-optimised
 * callee may.  GCC 16.2: "2 100 3 4 / 2 100 3 4 / 12 1100 31 41".
 */
#include <stdio.h>

struct S { long v[4]; };
struct S sa_make(const struct S *p);

__asm__(".text\n"
        ".globl sa_make\n"
        ".type sa_make,@function\n"
        "sa_make:\n"
        "  movq (%rsi), %rax\n"
        "  addq $1, %rax\n"
        "  movq %rax, (%rdi)\n"      /* slot.v[0] = p->v[0] + 1 (written first) */
        "  movq (%rsi), %rax\n"      /* re-read p->v[0]: legal, slot != *p     */
        "  imulq $100, %rax\n"
        "  movq %rax, 8(%rdi)\n"     /* slot.v[1] = p->v[0] * 100              */
        "  movq 16(%rsi), %rax\n"
        "  movq %rax, 16(%rdi)\n"
        "  movq 24(%rsi), %rax\n"
        "  movq %rax, 24(%rdi)\n"
        "  movq %rdi, %rax\n"
        "  ret\n"
        ".size sa_make, .-sa_make\n");

__attribute__((noinline)) static void show(const struct S *s) {
    printf("%ld %ld %ld %ld\n", s->v[0], s->v[1], s->v[2], s->v[3]);
}

int main(void) {
    struct S x = {{1, 2, 3, 4}};
    x = sa_make(&x);                 /* address-taken destination */
    show(&x);

    struct S y = {{1, 2, 3, 4}};
    struct S *alias = &y;            /* escaped through a pointer */
    y = sa_make(alias);
    show(&y);

    struct S z[2] = {{{10, 20, 30, 40}}, {{11, 21, 31, 41}}};
    z[0] = sa_make(&z[1]);           /* destination unrelated to the argument */
    z[0].v[2] = z[1].v[2];
    z[0].v[3] = z[1].v[3];
    show(&z[0]);
    return 0;
}
