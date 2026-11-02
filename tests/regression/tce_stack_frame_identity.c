#include <stdio.h>

__attribute__((noinline)) static int descend(int depth, const int *previous) {
    int current = depth;
    if (depth == 0)
        return *previous;
    /* This syntactically tail-recursive call must retain distinct frames:
       `previous` points at the caller's automatic object. */
    return descend(depth - 1, &current);
}

int main(void) {
    int result = descend(100, (const int *)0);
    printf("%d\n", result);
    return result != 1;
}
