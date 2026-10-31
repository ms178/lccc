/* A non-tail recursion deep enough that the compile-time constant evaluator
 * cannot finish it: `deep(100000)` nests 100,000 levels in the interpreter,
 * `ackermann(3, 11)` (the deepest fold the pass is meant to serve) nests
 * ~16,400. `MAX_EVAL_CALL_DEPTH` therefore refuses this call and leaves it as
 * a call instead of recursing until the pipeline's 64 MiB stack is gone —
 * which aborted the compiler outright before the depth bound existed.
 *
 * The seed is `volatile` so nothing but the pass under test can fold it.
 */
#include <stdio.h>

static int deep(int n) {
    if (n == 0) return 0;
    return 1 + deep(n - 1);
}

int main(void) {
    volatile int n = 100000;
    volatile int result = deep(n);
    printf("%d\n", result);
    return 0;
}
