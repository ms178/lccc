/* GCC compatibility: `return expr;` inside a void function.
 *
 * glibc uses `return helper();` shims throughout (malloc.c tcache paths,
 * qsort.c, libio strops.c, login/setutxent.c). GCC in gnu mode accepts a
 * void-typed expression silently (pedwarn only under -pedantic) and treats
 * a VALUE return from a void function as a default-on warning with the
 * value discarded. lccc made both hard errors, then after the sema
 * relaxation the LOWERER emitted Return(Some(v)) where v was the never-
 * defined "result" of a void call — glibc __libc_free ICE'd with the
 * backend's no-home hard gate.
 *
 * Checks: the void-expression form runs the side effect exactly once per
 * call and returns properly on both the early-return and fallthrough
 * paths.
 */
#include <stdio.h>

static int calls;

static void helper(void) { calls++; }

static void outer(int c)
{
    if (c)
        return helper(); /* GNU: evaluated for side effects, then return */
    helper();
    helper();
}

int main(void)
{
    outer(1); /* +1 */
    outer(0); /* +2 */
    printf("calls=%d\n", calls);
    return calls == 3 ? 0 : 1;
}
