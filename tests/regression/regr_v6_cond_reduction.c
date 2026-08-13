// regr_v6_cond_reduction.c
//
// The -O2 reduction vectorizer (src/passes/vectorize.rs) matched
// `for (i...) { if (cond) s += a[i]; else s -= k; }` as a "simple sum"
// pattern, then rewrote the loop into an 8-wide vector loop while keeping
// the scalar data-dependent branch: whenever the scalar condition was true
// it added ALL 8 lanes, and on the false path it copied the vector value
// computed on the other branch (undefined). Even without an else arm,
// `if (a[i] & 1) s += a[i];` produced garbage whenever the first element of
// a group of 8 was even, and totally wrong results for all-even data.
//
// The analyzer now rejects any reduction whose accumulator is updated
// conditionally (the update block must dominate the latch) or written from
// more than one place, so this program must produce the exact reference
// sums.
#include <stdio.h>

__attribute__((noinline)) static int f2(int *a, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) {
        if (a[i] & 1) s += a[i];
        else s -= 2;
    }
    return s;
}

__attribute__((noinline)) static int f3(int *a, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) {
        if (a[i] & 1) s += a[i];
    }
    return s;
}

int main(void) {
    int a[8000];
    int bad = 0;

    // mixed parity
    long exp = 0;
    for (int i = 0; i < 8000; i++) {
        a[i] = 1 + (i % 7);
        if (a[i] & 1) exp += a[i];
        else exp -= 2;
    }
    int r = f2(a, 8000);
    if (r != exp) { printf("MISMATCH mixed: got=%d exp=%ld\n", r, exp); bad = 1; }

    // all odd
    long exp2 = 0;
    for (int i = 0; i < 8000; i++) { a[i] = 101; exp2 += a[i]; }
    int r2 = f2(a, 8000);
    if (r2 != exp2) { printf("MISMATCH allodd: got=%d exp=%ld\n", r2, exp2); bad = 1; }

    // all even -> every iteration takes the else arm (s -= 2)
    long exp3 = 0;
    for (int i = 0; i < 8000; i++) { a[i] = 100; exp3 -= 2; }
    int r3 = f2(a, 8000);
    if (r3 != exp3) { printf("MISMATCH alleven: got=%d exp=%ld\n", r3, exp3); bad = 1; }

    // no-else variant, all even
    long exp4 = 0;
    for (int i = 0; i < 8000; i++) { a[i] = 100; }
    int r4 = f3(a, 8000);
    if (r4 != exp4) { printf("MISMATCH noelse-alleven: got=%d exp=%ld\n", r4, exp4); bad = 1; }

    if (bad) return 1;
    printf("ok %d %d %d %d\n", r, r2, r3, r4);
    return 0;
}
