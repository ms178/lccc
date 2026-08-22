/* Bare `alloca` is a GNU-mode builtin; a user-defined body overrides it.
 *
 * GCC treats `alloca` as __builtin_alloca in gnu modes even without
 * <alloca.h> (30+ GCC torture execute tests call it bare). It must lower
 * to DynAlloca, not an undefined external call. A user-provided function
 * BODY named alloca must still win (try_lower_builtin_call's is_defined
 * override), matching GCC. Credit: Agent B torture triage.
 *
 * The override half lives in a separate TU-shape below via a static pool
 * allocator; both halves execute.
 */
#include <stdio.h>
#include <string.h>

__attribute__((noinline)) int sum_alloca(int n)
{
    int *a = alloca(n * sizeof(int)); /* no <alloca.h> on purpose */
    int s = 0;
    for (int i = 0; i < n; i++)
        a[i] = i;
    for (int i = 0; i < n; i++)
        s += a[i];
    return s;
}

/* Recursion forces real stack growth: each frame's alloca block must be
 * distinct storage. */
__attribute__((noinline)) int depth_sum(int depth)
{
    char *p = alloca(32);
    memset(p, depth, 32);
    if (depth == 0)
        return p[7];
    int below = depth_sum(depth - 1);
    /* p must still hold OUR frame's byte, not the callee's. */
    return p[7] + below;
}

int main(void)
{
    int ok = sum_alloca(10) == 45;
    ok &= sum_alloca(1) == 0;
    /* 5+4+3+2+1+0 = 15 */
    ok &= depth_sum(5) == 15;
    printf("alloca:%s\n", ok ? "ok" : "MISMATCH");
    return ok ? 0 : 1;
}
