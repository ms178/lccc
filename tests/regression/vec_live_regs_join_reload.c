/* vec_live_regs cross-block elision (chorba_sse41 join-edge stale-scratch
 * shape): block A ends with a vector op whose result is claimed by a
 * scratch register; the conditional branch enters block B, whose own vector
 * op reuses the same scratch; after the join the value A produced is
 * consumed again. The reg-live claim map is block-local, so the join-edge
 * consumer must reload from the stack home, not trust the scratch.
 * Requires CCC_ENABLE_VECREG (see .env). Differential vs GCC. */
#include <stdio.h>

int a[64], b[64], c[64], d[64];

__attribute__((noinline)) static int run(int mode) {
    for (int i = 0; i < 64; i++) {
        a[i] = b[i] * 3 + 7;
    }
    int t = 0;
    if (mode & 1) {
        for (int i = 0; i < 64; i++) {
            c[i] = b[i] * 5 - 3;
        }
        t += c[3];
    } else {
        for (int i = 0; i < 64; i++) {
            d[i] = b[i] << 2;
        }
        t -= d[5];
    }
    /* Consume A's product AFTER the join. */
    for (int i = 0; i < 64; i++) {
        t += a[i];
    }
    return t;
}

int main(void) {
    for (int i = 0; i < 64; i++)
        b[i] = i * 11 - 5;
    int r0 = run(0);
    int r1 = run(1);
    printf("r0=%d r1=%d\n", r0, r1);
    return (r0 == r1) ? 1 : 0;
}
