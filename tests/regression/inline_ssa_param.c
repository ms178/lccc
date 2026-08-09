/* Regression: inliner must substitute pure scalar parameters with the call
 * argument (SSA substitution), eliminating the store-into-home + load
 * round-trip. The loop must still read the ORIGINAL argument (not 0 or a
 * stale home), and params that are modified or address-taken must stay
 * memory-backed.
 */
#include <stdint.h>
#include <stdio.h>

static int desc_loop(int n) {
    int a = 0;
    for (int i = n - 1; i >= 0; i--) a = (a + i) & 0x7ffff;
    return a;
}

static int modified_param(int n) {
    int sum = 0;
    while (n > 0) { sum += n; n--; }  /* n is modified -> must stay memory-backed */
    return sum;
}

static int addr_taken(int *p) { return *p * 2; }
static int caller_addr(void) {
    int v = 21;
    return addr_taken(&v);  /* address of a local, not a param home */
}

int main(void) {
    int x = 8;
    if (desc_loop(x) != 28) { printf("FAIL desc_loop=%d\n", desc_loop(x)); return 1; }
    if (desc_loop(0) != 0) { printf("FAIL desc_loop(0)=%d\n", desc_loop(0)); return 2; }
    if (modified_param(5) != 15) { printf("FAIL modified=%d\n", modified_param(5)); return 3; }
    if (caller_addr() != 42) { printf("FAIL addr_taken\n"); return 4; }
    printf("INLINE-SSA-PARAM-OK\n");
    return 0;
}
