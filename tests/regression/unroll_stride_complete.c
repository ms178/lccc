/* Constant-stride complete unrolling (found via the dot8 godbolt gap).
 *
 * Both complete unrollers (two-block and general) historically rejected
 * `iv_step != 1`, so `for (i = 0; i < 8; i += 4)` — the classic 4-way
 * software-pipelined accumulator shape — never fully unrolled, and Pass A
 * routed 2-block loops away from the general cloner, leaving the stride
 * loop as a loop while gcc/clang/icx all straightened it. dot8 at
 * -O3 -march=x86-64-v3: lccc 46 instructions vs gcc 21 / clang 14 /
 * icx 10, with 6 IV-bump instructions per iteration carrying the loop.
 *
 * The fix computes the exact static trip count for any non-zero stride
 * (ceil for `<`/`>`, floor+1 for `<=`/`>=`, sign-consistent), and the
 * cloner already substituted `init + k*step` per iteration. These kernels
 * pin the runtime results of every stride arm: exact division, ceil on a
 * non-divisible span, `<=` with and without residue, trip-1 (must stay
 * correct, unroll or not), countdown strides, and the FP accumulator
 * shape that motivated the fix (FMA-contracted at -O3 on v3 targets).
 */
#include <stdio.h>

long la[16], lb[16];
double da[16], db[16];
volatile double sink;

static void init(void) {
    for (int i = 0; i < 16; i++) {
        la[i] = i * 3 - 7;
        lb[i] = (i % 5) - 2;
        da[i] = i * 0.5 - 1.25;
        db[i] = (i % 4) * 0.75 + 0.125;
    }
}

/* dot8's shape: stride 4, exact division, 2 trips. */
static long dot8i(void) {
    long s0 = 0, s1 = 0, s2 = 0, s3 = 0;
    for (int i = 0; i < 8; i += 4) {
        s0 += la[i] * lb[i];
        s1 += la[i + 1] * lb[i + 1];
        s2 += la[i + 2] * lb[i + 2];
        s3 += la[i + 3] * lb[i + 3];
    }
    return (s0 + s1) + (s2 + s3);
}

static double dot8d(void) {
    double s0 = 0.0, s1 = 0.0, s2 = 0.0, s3 = 0.0;
    for (int i = 0; i < 8; i += 4) {
        s0 += da[i] * db[i];
        s1 += da[i + 1] * db[i + 1];
        s2 += da[i + 2] * db[i + 2];
        s3 += da[i + 3] * db[i + 3];
    }
    return (s0 + s1) + (s2 + s3);
}

/* Stride 3 over a non-divisible span: i = 0,3,6,9 (ceil). */
static long s3_10(void) {
    long s = 0;
    for (int i = 0; i < 10; i += 3)
        s += la[i] * i;
    return s;
}

/* Odd limit, stride 2: i = 0,2,4,6. */
static long s2_7(void) {
    long s = 0;
    for (int i = 0; i < 7; i += 2)
        s += la[i] * i - lb[i];
    return s;
}

/* `<=` exact (0,3,6,9) and with residue (last still 9). */
static long sle_9(void) {
    long s = 0;
    for (int i = 0; i <= 9; i += 3)
        s += la[i] * i;
    return s;
}

static long sle_10(void) {
    long s = 0;
    for (int i = 0; i <= 10; i += 3)
        s += la[i] * i;
    return s;
}

/* Single iteration: trip 1 falls outside the unroll bound; whichever way
 * it lowers, the result must be one body execution. */
static long t1(void) {
    long s = 0;
    for (int i = 0; i < 5; i += 5)
        s += la[i] * i;
    return s;
}

/* Stride 4, trip 4, both arrays read. */
static long d16_4(void) {
    long s = 0;
    for (int i = 0; i < 16; i += 4)
        s += la[i] + lb[i];
    return s;
}

/* Countdown stride: i = 12,10,8,6,4,2. */
static long down2(void) {
    long s = 0;
    for (int i = 12; i > 0; i -= 2)
        s += la[i] * i;
    return s;
}

/* Non-zero init with stride: i = 2,5,8. */
static long init2(void) {
    long s = 0;
    for (int i = 2; i < 11; i += 3)
        s += la[i] * lb[i];
    return s;
}

int main(void) {
    init();
    sink = dot8d();
    printf("%ld %.6g %ld %ld %ld %ld %ld %ld %ld %ld\n",
           dot8i(), sink, s3_10(), s2_7(), sle_9(), sle_10(), t1(), d16_4(),
           down2(), init2());
    return 0;
}
