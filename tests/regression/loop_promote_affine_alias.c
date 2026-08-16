/* Adversarial tests for the affine marching-pointer alias analysis in
 * loop_memory_promote (levkropp 57ab50aa backport). Each case constructs a
 * store sequence that LOOKS like it marches away from a loop-invariant
 * candidate but actually aliases it — the promotion must not fire, or must
 * preserve exact semantics if it does. */
#include <stdio.h>
#include <string.h>

/* 1. nbody shape (the intended win): j marches UP from i+1, never touches
 *    bodies[i] — promotion is legal; verify numeric identity. */
struct B { double x, y, z, vx, vy, vz, m; };
static struct B bod[5];
static double __attribute__((noinline)) nbody_step(void) {
    for (int i = 0; i < 5; i++) { bod[i].x = i; bod[i].vx = i * 0.5; bod[i].m = 1.0 + i; }
    for (int i = 0; i < 4; i++) {
        double ax = 0;
        for (int j = i + 1; j < 5; j++) {          /* stores march up */
            double d = bod[j].x - bod[i].x + 0.5;
            ax += d * bod[j].m;
            bod[j].vx += d * 0.001;                 /* store to bod[j].vx */
        }
        bod[i].vx += ax * 0.01;                     /* read+write bod[i].vx */
    }
    double s = 0;
    for (int i = 0; i < 5; i++) s += bod[i].vx;
    return s;
}

/* 2. Store marches DOWN and lands exactly ON the candidate in the last
 *    iteration: j from N-1 down to i — aliases bod2[i] at j==i. */
static long arr2[16];
static long __attribute__((noinline)) march_down_hits(int i) {
    for (int k = 0; k < 16; k++) arr2[k] = k;
    long acc = 0;
    for (int j = 15; j >= i; j--) {                /* marches down TO i */
        acc += arr2[i];                             /* candidate load */
        arr2[j] = 100 + j;                          /* store hits arr2[i] at j==i */
    }
    return acc + arr2[i];
}

/* 3. March step larger than element: stride 2 over longs, candidate at odd
 *    index between the strided stores — truly disjoint, promotion legal. */
static long arr3[33];
static long __attribute__((noinline)) strided_disjoint(void) {
    for (int k = 0; k < 33; k++) arr3[k] = k;
    long acc = 0;
    for (int j = 0; j < 16; j++) {
        acc += arr3[32];                            /* invariant candidate (top) */
        arr3[2 * j] = j;                            /* even slots only, < 32 */
    }
    return acc + arr3[32];
}

/* 4. Same symbolic base, DIFFERENT outer IV coefficient: stores to m[j][i],
 *    candidate m[i][j] — rows vs columns cross at the diagonal. */
static long m4[8][8];
static long __attribute__((noinline)) row_col_cross(int i) {
    for (int r = 0; r < 8; r++) for (int c = 0; c < 8; c++) m4[r][c] = r * 8 + c;
    long acc = 0;
    for (int j = 0; j < 8; j++) {
        acc += m4[i][i];                            /* invariant during j loop */
        m4[j][i] = -1;                              /* hits m4[i][i] at j==i */
    }
    return acc;
}

int main(void) {
    double n = nbody_step();
    /* reference 6.075000 verified identical across gcc -O2 (x86+aarch64) */
    if (n < 6.074999 || n > 6.075001) { printf("FAIL nbody %.6f\n", n); return 1; }
    /* march_down_hits(3): j=15..3, acc sums arr2[3]13 times; store at j==3
       sets arr2[3]=103. Iterations j>3 read original 3; the j==3 iteration
       reads 3 BEFORE storing. acc = 13*3 = 39; final arr2[3] = 103. */
    long v = march_down_hits(3);
    if (v != 39 + 103) { printf("FAIL march_down %ld\n", v); return 2; }
    long s = strided_disjoint();
    if (s != 16 * 32 + 32) { printf("FAIL strided %ld\n", s); return 3; }
    /* row_col_cross(2): j=0..7; at j==2 the store sets m4[2][2]=-1.
       Reads: j=0,1,2 see original m4[2][2]=18 (load before store at j==2),
       j=3..7 see -1. acc = 3*18 + 5*(-1) = 49. */
    long r = row_col_cross(2);
    if (r != 49) { printf("FAIL rowcol %ld\n", r); return 4; }
    printf("OK\n");
    return 0;
}
