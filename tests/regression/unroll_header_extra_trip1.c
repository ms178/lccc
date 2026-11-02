/* Complete-unroll with header-extra side effects AND a single-trip loop.
 *
 * A loop condition comma-expression (`for (i=0; cnt++, i<N; i++)`) evaluates
 * `trip + 1` times (the failing evaluation included).  A red-team audit of
 * the "delete 1-trip loops" feature found its first victim: for N == 1 the
 * deletion path kept the original header (one `cnt++`) and dropped the
 * second evaluation, producing cnt == 1 instead of 2.  The merged general
 * cloner now emits the final guard for trip == 1 too: the guard re-runs the
 * header's extra instructions once more with the failing-entry environment,
 * so cnt == 2.  The trip-4 variant exercises the same machinery on the
 * multi-clone path (cnt == 5).
 */
#include <stdio.h>
int A[64];
static void init(void) { for (int i = 0; i < 64; i++) A[i] = i * 7 - 20; }
#define NOINLINE __attribute__((noinline))
NOINLINE int k_header_effect1(void) {
    int cnt = 0, s = 0;
    for (int i = 0; cnt++, i < 1; i++) s += A[i];
    return s * 100 + cnt;
}
NOINLINE int k_header_effect4(void) {
    int cnt = 0, s = 0;
    for (int i = 0; cnt++, i < 4; i++) s += A[i];
    return s * 100 + cnt;
}
/* Two comma-separated counters + trip 1. */
NOINLINE int k_two_counter1(void) {
    int a = 0, b = 0, s = 0;
    for (int i = 0; a++, b += 2, i < 1; i++) s += A[i];
    return s * 1000 + a * 10 + b;
}
/* trip=2 (clone path) with a counter: condition runs 3 times. */
NOINLINE int k_header_effect2(void) {
    int cnt = 0, s = 0;
    for (int i = 0; cnt++, i < 2; i++) s += A[i];
    return s * 100 + cnt;
}
int main(void) {
    init();
    printf("e1=%d (ref %d)\n", k_header_effect1(), A[0] * 100 + 2);
    printf("e4=%d (ref %d)\n", k_header_effect4(), (A[0] + A[1] + A[2] + A[3]) * 100 + 5);
    printf("t1=%d (ref %d)\n", k_two_counter1(), A[0] * 1000 + 12);
    printf("e2=%d (ref %d)\n", k_header_effect2(), (A[0] + A[1]) * 100 + 3);
    return 0;
}
