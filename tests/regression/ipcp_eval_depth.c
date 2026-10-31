/* Exercise the constant evaluator with actual constant arguments.  The small
 * recursion must fold; the larger one exceeds MAX_EVAL_CALL_DEPTH and must
 * remain a runtime call.  The volatile result keeps the latter observable,
 * without hiding its argument from the evaluator. */
#include <stdio.h>

__attribute__((noinline)) static int deep(int n) {
    if (n == 0) return 0;
    return 1 + deep(n - 1);
}

int main(void) {
    int small = deep(100);
    volatile int result = deep(100000);
    printf("%d %d\n", small, result);
    return 0;
}
