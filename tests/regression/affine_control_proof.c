/* Compile with -fwrapv. A unique exit is not necessarily must-execute, and
   its own expression must not wrap before another comparison can borrow its
   induction range. Before the repair these return 2/95 instead of 5/97. */
#include <stdio.h>

__attribute__((noinline)) static int optional_guard(void) {
    int count = 0;
    for (int i = 0; ; i += 1073741823) {
        if (i > 0 && i != 1073741823) {
            if (i + 2 < 100) { } else return count;
        }
        ++count;
    }
}

__attribute__((noinline)) static int wrapping_guard(void) {
    int count = 0, sum = 0;
    for (int i = 0; i + 1073741830 < 2147483644; i += 1073741823) {
        sum += (i + 100 < 1000);
        ++count;
    }
    return 10 * count + sum;
}

int main(void) {
    int a = optional_guard(), b = wrapping_guard();
    printf("affine-control-proof %d %d\n", a, b);
    return a != 5 || b != 97;
}
