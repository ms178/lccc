/* A data object whose name is a library function name shadows that function
 * (C11 6.2.1p4), at EVERY scope, and the builtin fold table must stay silent
 * for it.
 *
 * The defect this pins: the lowerer's `is_func_ptr_variable` exempted file-scope
 * objects that also appeared in `known_functions`, and sema's constant-expression
 * path had no shadow test at all. `known_functions` holds every function name
 * sema reported -- including the implicit library prototypes that
 * `declare_implicit_functions` pre-declares -- so EVERY file-scope pointer object
 * named after a library function was exempted, i.e. the veto swallowed exactly
 * the case it existed for. Three symptoms, all measured against gcc 14.2:
 *
 *   - in an array bound `char vla[strlen("abc") == 103 ? 1 : -1]`, sema folded
 *     `strlen("abc")` to 3 and rejected the TU with `size of array is negative`
 *     where the bound is a VLA of runtime size 1;
 *   - in a value expression, a constant folded through the builtin
 *     (`strlen("abc")` -> 3, `abs(-5)` -> 5) where the program must call the
 *     pointer object (witnesses 103 and 75 below);
 *   - at -O0, `call strlen` was emitted against a `size_t (*)(const char *)`
 *     OBJECT, which the assembler bound to the object's address, so the program
 *     jumped into its own data: SIGSEGV (exit 139) where gcc exits 0 (pinned
 *     separately by builtin_call_shadowed_by_function_pointer_o0.c so the
 *     compile-time failure on the VLA row does not mask the -O0 crash).
 *
 * No <string.h>/<stdlib.h> on purpose: including them would declare these names
 * as functions and make the object declarations a 6.7p4 conflict in every
 * compiler.
 */
#include <stdio.h>

static size_t my_strlen(const char *s) { size_t n = 0; while (s[n]) ++n; return 100 + n; }
static int my_abs(int v) { return 70 + (v < 0 ? -v : v); }
static void *my_memcpy(void *d, const void *s, size_t n)
{
    unsigned char *dp = d; const unsigned char *sp = s;
    for (size_t i = 0; i < n; ++i) dp[i] = sp[i];
    return dp;
}
static int my_memcmp(const void *a, const void *b, size_t n)
{
    const unsigned char *p = a, *q = b;
    for (size_t i = 0; i < n; ++i) if (p[i] != q[i]) return 50 + (int)i;
    return 50;
}
static int my_strcmp(const char *a, const char *b)
{
    for (; *a && *a == *b; ++a, ++b) {}
    return 30 + (int)(unsigned char)*a - (int)(unsigned char)*b;
}

/* 1. file-scope object, internal linkage. External linkage is deliberately NOT
 *    exercised here, and the reason is measured, not stylistic: for a `.globl`
 *    object named `strlen`, gcc 14.2 -O2 emits `call *strlen(%rip)` against
 *    `.type strlen, @object`, the dynamic lookup binds glibc's *versioned*
 *    `strlen@@GLIBC_2.2.5` instead of this object, and the program loads eight
 *    bytes of libc code and jumps into them -- SIGSEGV, exit 139, with no output
 *    at all (gcc -O0/-O1 print 103 and exit 0). A differential corpus cannot use
 *    a shape its own oracle crashes on, so the external-linkage variant is
 *    documented here rather than tested. */
static size_t (*strlen)(const char *) = my_strlen;
/* 2. file-scope object, internal linkage (try_fold_builtin_or_attr_call path) */
static int (*abs)(int) = my_abs;
/* 3. file-scope object reached through a typedef'd pointer type */
typedef void *(*copier)(void *, const void *, size_t);
static copier memcpy = my_memcpy;
/* 4. file-scope compare objects */
static int (*memcmp)(const void *, const void *, size_t) = my_memcmp;
static int (*strcmp)(const char *, const char *) = my_strcmp;

/* 5. a parameter of pointer type shadows the name inside the function */
__attribute__((noinline)) static int through_parameter(int (*abs)(int), int v)
{
    return abs(v);
}

/* 6. a block-scope object shadows the name inside the block */
__attribute__((noinline)) static int through_block_scope(int v)
{
    int (*abs)(int) = my_abs;
    return abs(v) + 1;
}

/* 7. a block-scope `static` pointer object shadows the name inside the block */
__attribute__((noinline)) static size_t through_block_static(const char *s)
{
    static size_t (*strlen)(const char *) = my_strlen;
    return strlen(s) + 2;
}

int main(void)
{
    char buf[8] = {0};
    /* Sema constant-expression guard witness: on baseline LCCC, sema folded
     * `strlen("abc")` to 3, folded `3 == 103 ? 1 : -1` to -1, and rejected this
     * declaration at compile time with `error: size of array is negative`. With
     * `strlen` shadowed by a pointer object, this is a VLA of runtime size 1. */
    char vla[strlen("abc") == 103 ? 1 : -1];

    /* 103 = 100 + strlen("abc"); a folded builtin would print 3 */
    printf("strlen=%zu vla=%zu\n", strlen("abc"), sizeof(vla));
    /* 75 = 70 + |-5|; a folded builtin would print 5 */
    printf("abs=%d\n", abs(-5));
    /* the object copies and returns the destination */
    memcpy(buf, "xy", 3);
    printf("memcpy=%s\n", buf);
    /* 50 = "equal" witness; a folded builtin would print 0 */
    printf("memcmp_equal=%d\n", memcmp("ab", "ab", 2));
    /* 51 = first difference at index 1; a folded builtin would print a sign */
    printf("memcmp_diff=%d\n", memcmp("ax", "ab", 2));
    /* 30 + 'a' - 'a' = 30 for equal strings; a folded builtin would print 0 */
    printf("strcmp_equal=%d\n", strcmp("a", "a"));
    /* 5. parameter shadowing: 70 + 7 */
    printf("param=%d\n", through_parameter(my_abs, -7));
    /* 6. block scope: 70 + 7 + 1 */
    printf("block=%d\n", through_block_scope(-7));
    /* 7. block-scope static: 100 + 4 + 2 */
    printf("block_static=%zu\n", through_block_static("abcd"));
    /* the compiler's own interface is NOT shadowed by any of the above: a
     * declaration carries the bare basename, so __builtin_* still folds, and
     * gcc folds it identically (both print 3 and 5) */
    printf("builtin_strlen=%d builtin_abs=%d\n",
           (int)__builtin_strlen("abc"), __builtin_abs(-5));
    return 0;
}
