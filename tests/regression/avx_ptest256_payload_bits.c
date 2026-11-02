/* 256-bit integer PTEST (_mm256_test{z,c,nzc}_si256) observes EVERY payload
 * bit, unlike VTESTPS/PD (sign bits only). Probe each of the 256 bit
 * positions alone and against neighbours in both 128-bit lanes, plus a
 * deterministic random sweep, through the public wrappers and the raw GCC
 * builtins. AVX, not AVX2, is the minimum ISA for every operation below. */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

typedef long long v4di __attribute__((vector_size(32)));

static int wrong(int z, int c, int nzc, const unsigned long long *a,
                 const unsigned long long *b)
{
    unsigned long long and_ = 0, andn = 0;
    for (int i = 0; i < 4; ++i) {
        and_ |= a[i] & b[i];
        andn |= ~a[i] & b[i];
    }
    int ez = and_ == 0, ec = andn == 0;
    return z != ez || c != ec || nzc != (!ez && !ec);
}

__attribute__((noinline)) static int check(const unsigned long long *a,
                                           const unsigned long long *b)
{
    __m256i x = _mm256_loadu_si256((const __m256i *)a);
    __m256i y = _mm256_loadu_si256((const __m256i *)b);
    if (wrong(_mm256_testz_si256(x, y), _mm256_testc_si256(x, y),
              _mm256_testnzc_si256(x, y), a, b))
        return 1;
    v4di p, q;
    memcpy(&p, a, sizeof p);
    memcpy(&q, b, sizeof q);
    if (wrong(__builtin_ia32_ptestz256(p, q), __builtin_ia32_ptestc256(p, q),
              __builtin_ia32_ptestnzc256(p, q), a, b))
        return 2;
    return 0;
}

static unsigned long long state = 0x9e3779b97f4a7c15ull;
static unsigned long long next(void)
{
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    return state;
}

int main(void)
{
    unsigned long long a[4], b[4];
    unsigned cases = 0, z = 0, c = 0, nzc = 0;
    for (int i = 0; i < 256; ++i) {
        for (int k = 0; k < 4; ++k) {
            /* a = one bit; b = same bit, the bit 128 positions away (other
             * lane), the adjacent bit, and all-ones. */
            int j = k == 0 ? i : k == 1 ? (i + 128) & 255 : k == 2 ? i ^ 1 : -1;
            memset(a, 0, sizeof a);
            memset(b, 0, sizeof b);
            a[i / 64] = 1ull << (i % 64);
            if (j < 0)
                memset(b, 0xff, sizeof b);
            else
                b[j / 64] = 1ull << (j % 64);
            int r = check(a, b);
            if (r) {
                printf("FAIL kind=%d bit=%d other=%d\n", r, i, j);
                return 1;
            }
            __m256i x = _mm256_loadu_si256((const __m256i *)a);
            __m256i y = _mm256_loadu_si256((const __m256i *)b);
            z += _mm256_testz_si256(x, y);
            c += _mm256_testc_si256(x, y);
            nzc += _mm256_testnzc_si256(x, y);
            ++cases;
        }
    }
    for (int n = 0; n < 4096; ++n) {
        for (int k = 0; k < 4; ++k) {
            a[k] = next();
            /* Sparse b makes all three flag outcomes reachable. */
            b[k] = next() & next() & next() & next();
            if (n & 1)
                b[k] &= a[k];
            if (n & 2)
                b[k] &= ~a[k];
        }
        int r = check(a, b);
        if (r) {
            printf("FAIL kind=%d random=%d\n", r, n);
            return 1;
        }
        __m256i x = _mm256_loadu_si256((const __m256i *)a);
        __m256i y = _mm256_loadu_si256((const __m256i *)b);
        z += _mm256_testz_si256(x, y);
        c += _mm256_testc_si256(x, y);
        nzc += _mm256_testnzc_si256(x, y);
        ++cases;
    }
    printf("ptest256 cases=%u z=%u c=%u nzc=%u\n", cases, z, c, nzc);
    return 0;
}
