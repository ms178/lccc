/* the auto-vectorizer must NEVER vectorize loops touching
 * volatile objects (C11 6.7.3p7: a vector load/store observes volatile
 * memory with a different width/count/order than the source program).
 * Failing shape (pre-fix): volatile int array reduction at -O2 printed a
 * garbage sum (correctness test volatile_access: 884305999 instead of 15).
 * The vectorizer had zero volatile awareness; the legality gate
 * func_has_volatile_loop_access() was added in the fix. */
#include <stdio.h>

int main(void) {
    volatile int arr[8] = {1, 2, 3, 4, 5, 6, 7, 8};
    int sum = 0;
    for (int i = 0; i < 8; i++) sum += arr[i];
    if (sum != 36) { printf("FAIL sum=%d\n", sum); return 1; }
    printf("PASS volatile_loop\n");
    return 0;
}
