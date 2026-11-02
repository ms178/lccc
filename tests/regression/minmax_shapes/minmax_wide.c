/* Width-preserving reduction counters, including empty limits whose low
 * 32 bits are positive. Compare this complete program with the GCC oracle. */
#include <limits.h>
#include <stdio.h>

__attribute__((noinline))
static int extreme(const int *a, long long n, long long start, long long *out) {
    int m = INT_MIN;
    long long i = start;
    for (; i < n; ++i)
        if (a[i] > m) m = a[i];
    *out = i;
    return m;
}

/* Constant starts allow vectorization; the dynamic-start control must remain
 * correct regardless of admission, so both are checked against scalar math. */
__attribute__((noinline))
static int from_zero(const int *a, long long n, long long *out) {
    int m = INT_MIN;
    long long i = 0;
    for (; i < n; ++i)
        if (a[i] > m) m = a[i];
    *out = i;
    return m;
}

__attribute__((noinline))
static int from_one(const int *a, long long n, long long *out) {
    int m = INT_MIN;
    long long i = 1;
    for (; i < n; ++i)
        if (a[i] > m) m = a[i];
    *out = i;
    return m;
}

int main(void) {
    int a[81];
    for (int i = 0; i < 81; ++i) a[i] = ((i * 997) % 131) - 65;
    for (long long n = -3; n <= 80; ++n) {
        for (int start = 0; start <= 1; ++start) {
            long long out = -1, control = -1;
            int expected = extreme(a, n, start, &control);
            int got = start ? from_one(a, n, &out) : from_zero(a, n, &out);
            if (got != expected || out != control) return 1;
        }
    }
    const long long empty[] = {
        LLONG_MIN, LLONG_MIN + 17, -4294967296LL + 17, -2147483649LL, -1, 0
    };
    for (unsigned j = 0; j < sizeof(empty) / sizeof(empty[0]); ++j) {
        long long out = -1;
        if (from_zero(a, empty[j], &out) != INT_MIN || out != 0) return 2;
        if (from_one(a, empty[j], &out) != INT_MIN || out != 1) return 3;
    }
    puts("wide-counter min/max: PASS");
    return 0;
}
