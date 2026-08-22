/* Complete unrolling must replace uses of the loop IV after the loop with its
   final value. The loop body is intentionally empty (`continue`). */
#include <stdio.h>

__attribute__((noinline)) static int run(void) {
    int i;
    for (i = 0; i < 10; ++i)
        continue;
    return i;
}

int main(void) {
    int result = run();
    printf("%d\n", result);
    return result != 10;
}
