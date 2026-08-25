/* A nested non-local goto bypasses the child's epilogue. The child allocator
 * must not use callee-saved GPR homes that would overwrite parent live values
 * before rbp/rsp are restored. */
#include <stdlib.h>

__attribute__((noinline))
static int exercise(int input, int jump)
{
    __label__ target;

    void transfer(int enabled)
    {
        if (enabled)
            goto target;
    }

    int carried = input + 2;
    transfer(jump);
target:
    return carried;
}

int main(void)
{
    if (exercise(1, 1) != 3 || exercise(2, 1) != 4)
        abort();
    return 0;
}
