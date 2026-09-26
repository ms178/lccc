/* I64/U64 elementwise maps use two XMM lanes even under AVX2.  Exercise
 * zero/single/odd/even trip counts, wraparound, invariant broadcasts and
 * dependence-distance fallback.  The i686 backend must leave these scalar. */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define N 259
static uint64_t a[N], b[N], out[N], ref[N];
static uint64_t overlap[N + 8], shadow[N + 8];

__attribute__((noinline)) void sub64(uint64_t *restrict d, const uint64_t *restrict x,
                                      const uint64_t *restrict y, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) d[i] = x[i] - y[i];
}
__attribute__((noinline)) void rsub64(uint64_t *restrict d, const uint64_t *restrict x,
                                       uint64_t k, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) d[i] = k - x[i];
}
__attribute__((noinline)) void add64(uint64_t *restrict d, const uint64_t *restrict x,
                                      const uint64_t *restrict y, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) d[i] = x[i] + y[i];
}
/* Without restrict, writing ahead of an input read must take the scalar
 * path.  In-place (distance zero) is legal to vectorize. */
__attribute__((noinline)) void shifted64(uint64_t *d, const uint64_t *x,
                                          unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) d[i] = x[i] - 4u;
}

static int check(const char *what, unsigned long n) {
    for (unsigned long i = 0; i < N; ++i)
        if (out[i] != ref[i]) {
            printf("FAIL %s n=%lu i=%lu got=%llu want=%llu\n", what, n, i,
                   (unsigned long long)out[i], (unsigned long long)ref[i]);
            return 1;
        }
    return 0;
}
int main(void) {
    for (unsigned long i = 0; i < N; ++i) {
        a[i] = UINT64_MAX - (uint64_t)i * UINT64_C(0x123456789abcdef);
        b[i] = ((uint64_t)i + 17u) * UINT64_C(0xfedcba987654321);
    }
    for (unsigned long n = 0; n <= N; ++n) {
        for (unsigned long i = 0; i < N; ++i)
            ref[i] = UINT64_C(0xa5a5a5a5a5a5a5a5);
        memcpy(out, ref, sizeof out);
        sub64(out, a, b, n);
        for (unsigned long i = 0; i < n; ++i) {
            volatile uint64_t x = a[i], y = b[i];
            ref[i] = x - y;
        }
        if (check("sub", n)) return 1;
        memcpy(out, ref, sizeof out);
        rsub64(out, a, UINT64_C(0xdeadbeef01234567), n);
        for (unsigned long i = 0; i < n; ++i) {
            volatile uint64_t x = a[i];
            ref[i] = UINT64_C(0xdeadbeef01234567) - x;
        }
        if (check("rsub", n)) return 1;
        memcpy(out, ref, sizeof out);
        add64(out, a, b, n);
        for (unsigned long i = 0; i < n; ++i) {
            volatile uint64_t x = a[i], y = b[i];
            ref[i] = x + y;
        }
        if (check("add", n)) return 1;
    }
    for (unsigned long offset = 0; offset < 5; ++offset) {
        for (unsigned long n = 0; n < 32; ++n) {
            for (unsigned long i = 0; i < N + 8; ++i)
                shadow[i] = overlap[i] = (uint64_t)i * UINT64_C(0x5555555500112233);
            uint64_t *d = overlap + 3 + offset;
            const uint64_t *x = overlap + 4;
            uint64_t *expected = shadow + 3 + offset;
            const uint64_t *input = shadow + 4;
            shifted64(d, x, n);
            for (unsigned long i = 0; i < n; ++i) expected[i] = input[i] - 4u;
            for (unsigned long i = 0; i < N + 8; ++i)
                if (overlap[i] != shadow[i]) {
                    printf("FAIL alias shift=%lu n=%lu i=%lu\n", offset, n, i);
                    return 1;
                }
        }
    }
    puts("OK map_i64_two_lane");
    return 0;
}
