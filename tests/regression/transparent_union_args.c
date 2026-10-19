/* GCC transparent_union arguments (PR #870 record-tag gate root cause).
 * A parameter of transparent union type accepts any member's type and
 * travels with its FIRST member's ABI. lccc's parser dropped the attribute
 * on `union U {...} __attribute__((transparent_union));`, lowering then
 * rebuilt the layout unmarked, and a member argument reached the callee as
 * the address of a union: `a_bits(7)` faulted. A member whose type differs
 * from the first member's (`float` in {int; float}) must arrive as its bits
 * in the first member's register. Expected output is GCC 16.2's. */
#include <stdio.h>

union A { int i; float f; unsigned u; } __attribute__((transparent_union));
union P { long *l; const char *s; void (*fn)(void); } __attribute__((transparent_union));
union W { long l; int i; } __attribute__((transparent_union));
typedef union { int *ip; double *dp; } T __attribute__((transparent_union));

__attribute__((noinline)) unsigned a_bits(union A a) { return a.u; }
__attribute__((noinline)) long p_val(union P p) { return (long)p.l; }
__attribute__((noinline)) int w_low(union W w) { return w.i; }
__attribute__((noinline)) long t_val(T t) { return (long)t.ip; }

enum E { E5 = 5 };
static void h(void) {}

int main(void) {
    union A ua = { .i = 42 };
    long arr[2], lv;
    const char *str = "s";
    int iv;
    double dv;
    enum E e = E5;
    printf("%u %u %u %u %u %u\n", a_bits(7), a_bits(1.5f), a_bits(9u), a_bits('a'), a_bits(ua),
           a_bits(e));
    printf("%d %d %d %d %d %d\n", p_val(&lv) == (long)&lv, p_val(str) == (long)str, p_val(0) == 0,
           p_val((void *)0) == 0, p_val(h) == (long)h, p_val(arr) == (long)arr);
    printf("%d %d %d\n", w_low(-3), w_low(1L << 40 | 9),
           t_val(&iv) == (long)&iv && t_val(&dv) == (long)&dv);
    return 0;
}
