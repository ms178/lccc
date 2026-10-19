/* `?:` result types and null pointer constants (C11 6.3.2.3p3, 6.5.15p6),
 * compared with GCC 16.2 (the mismatch diagnostics are disabled in .flags:
 * lccc accepts these forms, so their types must still be GCC's).
 *
 * - A null pointer constant is an integer constant expression with value 0,
 *   optionally cast to void *: enumerators count (`(void *)(N - N)`), so the
 *   kernel's __is_constexpr(N) is 1 -- lccc said 0, sized a file-scope array
 *   through __builtin_choose_expr with the wrong arm and computed 0 for the
 *   runtime value. `(void *)(v * 0)` is not one.
 * - `(char *)0` is not one: incompatible object pointers give void *.
 * - pointer vs integer is the pointer type: `c ? x : p` once had type int
 *   and truncated the pointer arm to 32 bits.
 */
#include <stdint.h>
#include <stdio.h>

#define __is_constexpr(x) \
    (sizeof(int) == sizeof(*(8 ? ((void *)((long)(x) * 0l)) : (int *)8)))
#define const_or(x, d) __builtin_choose_expr(__is_constexpr(x), (x), (d))
#define T(x) _Generic((x), int: "int", long: "long", char *: "char*", \
                      void *: "void*", int *: "int*", default: "other")

enum { Z = 0, N = 4 };
static int sized[const_or(N * 2, 1)];

__attribute__((noinline)) int constexpr_probe(int v)
{
    return __is_constexpr(N) * 100 + __is_constexpr(N * 2 + 1) * 10
        + __is_constexpr(v) + const_or(N, -1) * 1000 + const_or(v, -1);
}

__attribute__((noinline)) uintptr_t select_int_or_pointer(int c, char *p, int x)
{
    char *r = c ? x : p;
    return (uintptr_t)r;
}

__attribute__((noinline)) long advance_mismatched(int c, int *a, char *b)
{
    return (long)((uintptr_t)((c ? a : b) + 1)
                  - (uintptr_t)(c ? (void *)a : (void *)b));
}

int main(int argc, char **argv)
{
    static char buf[16];
    char *p = argv[0];
    int *ip = 0;
    int a[2];

    printf("%d %zu\n", constexpr_probe(3), sizeof sized);
    printf("%s %s %s %s %zu %zu\n", T(argc ? 1 : p), T(argc ? p : 1L),
           T(argc ? Z : p), T(argc ? (void *)Z : ip), sizeof(argc ? 1 : p),
           sizeof(argc ? p : 1));
    printf("%s %s %s\n", T(argc ? (char *)0 : ip), T(argc ? (void *)(N - N) : ip),
           T(argc ? (void *)(argc * 0) : ip));
    int truncation_ok = select_int_or_pointer(0, buf, 7) == (uintptr_t)buf
        && select_int_or_pointer(1, buf, -1) == (uintptr_t)-1;
    printf("%d %ld %ld\n", truncation_ok, advance_mismatched(1, a, buf),
           advance_mismatched(0, a, buf));
    if (constexpr_probe(3) != 110 + 4000 - 1 || sizeof sized != 8 * sizeof(int))
        return 1;
    return truncation_ok && advance_mismatched(1, a, buf) == 1 ? 0 : 2;
}
