/*
 * The post-structural inline phase must preserve -Os/-Oz's size-aware policy.
 * medium_helper is deliberately above that policy's size threshold and has
 * three call sites.  Unrestricted post-inlining used to clone it three times,
 * silently undoing the primary -Os inline decision.
 */
volatile int postinline_sink;

__attribute__((noinline)) static int side_effect(int x)
{
    postinline_sink += x;
    return x * 3 + 1;
}

static int medium_helper(int x)
{
    int y = x;
    if (x & 1)
        y += side_effect(x + 1);
    if (x & 2)
        y ^= side_effect(x + 2);
    if (x & 4)
        y -= side_effect(x + 3);
    if (x & 8)
        y += side_effect(x + 4);
    return y;
}

int call_medium_three_times(int x)
{
    return medium_helper(x) + medium_helper(x + 1) + medium_helper(x + 2);
}

int main(void)
{
    int result = call_medium_three_times(5);
    return result == 31 && postinline_sink == 58 ? 0 : 1;
}
