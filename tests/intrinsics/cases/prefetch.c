/* __builtin_prefetch(): every hint form must behave as a pure hint.
 *
 * The builtin used to be lowered to nothing at all, which is semantically
 * correct but turned zstd's row-based match finders (which prefetch their
 * hash/tag rows) into serialised memory-latency chains. The instruction is
 * back, so this case pins the semantics that make it safe:
 *
 *   - all argument forms parse and run: default, rw only, locality, write;
 *   - the hinted address is still *evaluated* (side effects of the address
 *     expression must happen exactly once, before the hint);
 *   - an out-of-bounds-looking prefetch (one past the end, a null pointer)
 *     must not fault;
 *   - the hint must not change any observable value.
 *
 * Output is compared byte-for-byte against GCC by the runner, so any
 * evaluation-order or faulting-address mistake shows up immediately.
 */
#include <stdio.h>
#include <stdlib.h>

static int next_slot(int *p, int *calls) {
    (*calls)++;
    return *p;
}

int main(void) {
    int calls = 0;
    int n = 4096;
    int *a = (int *)malloc((size_t)n * sizeof(int));
    if (!a) return 2;
    for (int i = 0; i < n; i++) a[i] = i ^ 0x5a5a5a5a;

    /* Every documented hint form, inside a loop (the shape zstd uses). */
    long sum = 0;
    for (int i = 0; i < n; i++) {
        __builtin_prefetch(a + i + 16);            /* default: read, loc 3 */
        __builtin_prefetch(a + i + 32, 0);         /* read */
        __builtin_prefetch(a + i + 48, 1);         /* write intent */
        __builtin_prefetch(a + i + 64, 0, 0);      /* non-temporal */
        __builtin_prefetch(a + i + 80, 0, 1);
        __builtin_prefetch(a + i + 96, 0, 2);
        __builtin_prefetch(a + i + 112, 1, 0);     /* write + non-temporal */
        sum += a[i];
    }
    printf("sum=%ld\n", sum);

    /* The address expression must be evaluated exactly once. */
    int idx = 8;
    __builtin_prefetch(a + idx++, 0, 2);
    printf("idx=%d\n", idx);
    __builtin_prefetch(a + next_slot(&idx, &calls), 1);
    printf("calls=%d idx=%d\n", calls, idx);

    /* Hinting at addresses that are never dereferenced must not fault:
     * a prefetch is not a load. */
    __builtin_prefetch((void *)0, 0, 3);
    __builtin_prefetch((char *)a + (size_t)n * sizeof(int) + 4096, 1, 0);
    __builtin_prefetch((char *)a + (size_t)n * sizeof(int), 0, 1);
    printf("survived\n");

    free(a);
    return 0;
}
