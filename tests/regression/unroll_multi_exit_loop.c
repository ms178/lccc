/* Regression: loop unrolling must not apply to multi-exit loops.
 * `while (i < n && acc < 50)` has a second exit (acc >= 50) inside the body;
 * the unroller only re-plumbs the header exit and cloned body exits read
 * stale phi values (f(10) came back 0 instead of 45 at -O3).
 */
#include <stdio.h>

static int cond_loop(int n) {
    int i = 0, acc = 0;
    while (i < n && acc < 50) { acc += i; i++; }
    return acc;
}

static int or_loop(int n) {
    int i = 0, acc = 0;
    while (i < n || acc < 3) { acc += i; i++; }
    return acc;
}

static int plain_loop(int n) {
    int acc = 0;
    for (int i = 0; i < n; i++) acc += i;
    return acc;
}

int main(void) {
    if (cond_loop(10) != 45) { printf("FAIL cond_loop(10)=%d want 45\n", cond_loop(10)); return 1; }
    if (cond_loop(100) != 55) { printf("FAIL cond_loop(100)=%d want 55\n", cond_loop(100)); return 2; }
    if (cond_loop(0) != 0) { printf("FAIL cond_loop(0)=%d\n", cond_loop(0)); return 3; }
    if (or_loop(1) != 3) { printf("FAIL or_loop(1)=%d want 3\n", or_loop(1)); return 4; }
    if (or_loop(10) != 45) { printf("FAIL or_loop(10)=%d want 45\n", or_loop(10)); return 5; }
    if (plain_loop(1000) != 499500) { printf("FAIL plain_loop(1000)=%d\n", plain_loop(1000)); return 6; }
    printf("UNROLL-MULTIEXIT-OK\n");
    return 0;
}
