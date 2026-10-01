/* Affine countdown kernel with checksum main.
 *
 * The shape `for (i = 0; i + C < N; i++)` is what the affine exit-compare fold
 * (`fold_affine_exit_compares`) exists for: without it the backend
 * materialises `leaq C(%iv)` on every iteration (one extra ALU op per
 * iteration, and one more register live across the compare).  The counts are
 * written to a volatile sink so neither compiler can delete the loops, and the
 * inner loop is nested (rotation's Guard E refuses nested loops, so this
 * measures the standalone fold, not rotation).
 *
 * Measured (Callgrind, -O2 -march=x86-64-v3, per this program): GCC 16.2 emits
 * 5 instructions per inner iteration, lccc 5 after the fold and 6 before it.
 */
#include <stdio.h>

#define N 4096
#define REPS 2000

static unsigned char buf[N];
static volatile long sink;

static long scan_c4(void) {
    long total = 0;
    for (int rep = 0; rep < REPS; rep++) {
        long s = 0;
        for (int i = 0; i + 4 < N; i++)
            s += buf[i];
        total += s;
        buf[rep & (N - 1)] ^= (unsigned char)rep;
    }
    return total;
}

static long scan_c1(void) {
    long total = 0;
    for (int rep = 0; rep < REPS; rep++) {
        long s = 0;
        for (int i = 0; i + 1 < (N - 2); i++)
            s += buf[i];
        total += s;
        buf[(rep * 7) & (N - 1)] ^= 3;
    }
    return total;
}

static long scan_step3(void) {
    long total = 0;
    for (int rep = 0; rep < REPS; rep++) {
        long s = 0;
        for (int i = 1; i + 8 < N; i += 3)
            s += buf[i];
        total += s;
        buf[(rep * 5) & (N - 1)] ^= 5;
    }
    return total;
}

int main(void) {
    for (int i = 0; i < N; i++)
        buf[i] = (unsigned char)(i * 31 + 7);
    long a = scan_c4();
    long b = scan_c1();
    long c = scan_step3();
    sink = a + b + c;
    printf("%ld\n", a + b + c);
    return 0;
}
