/* Conditional-load sinking (SV-03) acceptance fixture.
 *
 * Three `cond ? *p : *q` shapes: a bare dereference, an indexed dereference
 * and an indexed dereference with a different constant chain per arm.  The
 * untaken pointer ALWAYS points at a PROT_NONE page, so any transform that
 * speculates the LOAD (rather than only the ADDRESS) faults here.  Addresses
 * may be computed eagerly; loads may not.
 *
 * Expected stdout is exactly "-13.50" for every compiler and every pass
 * state.  Values are exact binary fractions, so no FP contraction or excess
 * precision can perturb the sum.
 */
#include <stdio.h>
#include <sys/mman.h>
#include <unistd.h>

__attribute__((noinline)) float pick(const float *p, const float *q, int c) {
    return c ? *p : *q;
}

__attribute__((noinline)) float pick_idx(const float *p, const float *q, int c, long i) {
    return c ? p[i] : q[i];
}

__attribute__((noinline)) float pick_chain(const float *p, const float *q, int c, long i) {
    return c ? p[i + 1] : q[i + 2];
}

int main(void) {
    long pagesz = sysconf(_SC_PAGESIZE);
    if (pagesz <= 0) return 2;
    float *good = mmap(0, (size_t)pagesz, PROT_READ | PROT_WRITE,
                       MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    float *bad = mmap(0, (size_t)pagesz, PROT_NONE,
                      MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (good == MAP_FAILED || bad == MAP_FAILED) return 2;
    good[0] = 3.5f; good[1] = -7.25f; good[2] = 1.25f; good[3] = 8.0f;

    volatile float sum = 0.0f;
    sum += pick(good, bad, 1);          /* 3.50  — bad is untaken */
    sum += pick(bad, good, 0);          /* 3.50  — bad is untaken */
    sum += pick_idx(good, bad, 1, 1);   /* -7.25 */
    sum += pick_idx(bad, good, 0, 1);   /* -7.25 */
    sum += pick_chain(good, bad, 1, 0); /* -7.25 */
    sum += pick_chain(bad, good, 0, 0); /* 1.25  */
    printf("%.2f\n", (double)sum);
    return 0;
}
