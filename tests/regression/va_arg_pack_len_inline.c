/* VAPACK-LEN-1: count sentinels must be rewritten in the remapped namespace,
 * including zero extras. Each call site has a different SSA value offset.
 */
#include <stdio.h>
extern inline __attribute__((always_inline, gnu_inline))
int count_extras(int fixed, ...) {
    int n = __builtin_va_arg_pack_len();
    if (fixed > 0) return n;
    return 10 + __builtin_va_arg_pack_len();
}
extern inline __attribute__((always_inline, gnu_inline))
int nested_count(int fixed, ...) {
    return count_extras(fixed, 55, __builtin_va_arg_pack());
}
__attribute__((noinline)) int nested_counts(int x) {
    return nested_count(x) + 10*nested_count(x, 4)
        + 100*nested_count(x, 5, 6, 7);
}
__attribute__((noinline)) int counts(int x) {
    int a = count_extras(x);
    int b = count_extras(x, 4);
    int c = count_extras(x, 5, x, 7, 8);
    return a + 10*b + 100*c;
}
int main(void) {
    if (counts(1) != 410) return 1;
    if (counts(-1) != 1520) return 2;
    if (nested_counts(1) != 421) return 3;
    if (nested_counts(-1) != 1531) return 4;
    puts("va_arg_pack_len_inline: OK");
    return 0;
}
