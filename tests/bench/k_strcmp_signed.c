/* PF-15 signed-char strcmp carrier benchmark.
 *
 * Keep the comparison helper out of line so its byte carrier allocation is
 * exactly what we time.  The late mismatch keeps the signed zero-test and
 * signed equality pair hot without making the answer constant to the compiler.
 */
#include "bench.h"

#define N 768

#if defined(__GNUC__)
#define NOINLINE __attribute__((noinline))
#else
#define NOINLINE
#endif

static char left[N], right[N];

NOINLINE static int signed_strcmp_loop(const char *a, const char *b) {
    while (*a && *a == *b) {
        ++a;
        ++b;
    }
    return (int)(unsigned char)*a - (int)(unsigned char)*b;
}

void bench_setup(void) {
    for (int i = 0; i != N - 1; ++i) {
        char c = (char)('a' + (i * 17u) % 23u);
        left[i] = c;
        right[i] = c;
    }
    left[N - 1] = 0;
    right[N - 1] = 0;
    right[N - 9] = (char)(left[N - 9] + 1); /* late, nonzero mismatch */
}

unsigned long long bench_run(void) {
    unsigned long long sum = 0;
    for (int off = 0; off != 64; ++off)
        sum += (unsigned)signed_strcmp_loop(left + off, right + off);
    return sum;
}
