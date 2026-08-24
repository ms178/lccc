/* Capture analysis must visit declaration initializers inside a nested body.
 * Otherwise enclosing locals referenced only by `T x = helper(a, b)` are
 * mistaken for undefined globals instead of static-chain frame members. */
#include <stdlib.h>

static int same(const int *a, const int *b)
{
    return *a == *b;
}

int main(void)
{
    int left = 7;
    int right = 9;

    void check(int expected)
    {
        int observed = same(&left, &right);
        if (observed != expected)
            abort();
    }

    check(0);
    right = 7;
    check(1);
    return 0;
}
