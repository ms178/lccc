/* GCC's five-word builtin setjmp buffer stores frame pointer, resume PC, and
 * stack pointer. The intrinsic is returns-twice: local stores on the path to
 * builtin_longjmp must remain observable on resume. */
#include <stdlib.h>

static void *jump_buffer[5];

__attribute__((noinline))
static void jump_back(void)
{
    __builtin_longjmp(jump_buffer, 1);
}

__attribute__((noinline))
static int exercise(void)
{
    int state = 0;
    if (__builtin_setjmp(jump_buffer) == 0) {
        state = 73;
        jump_back();
    }
    return state;
}

int main(void)
{
    if (exercise() != 73)
        abort();
    return 0;
}
