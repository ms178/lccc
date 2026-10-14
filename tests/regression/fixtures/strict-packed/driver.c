#define _GNU_SOURCE
#include <fenv.h>
#include <limits.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>
typedef double (*kernel)(const double *, int, int);
#define DECL(name) extern double name(const double *, int, int); \
    extern double actual_##name(const double *, int, int)
DECL(packed_poly); DECL(packed_bits); DECL(scalar_shift);
static int same(double a, double b) {
    uint64_t x, y; memcpy(&x, &a, 8); memcpy(&y, &b, 8);
    return x == y || (isnan(a) && isnan(b));
}
int main(void) {
    kernel ref[] = {packed_poly, packed_bits, scalar_shift};
    kernel got[] = {actual_packed_poly, actual_packed_bits, actual_scalar_shift};
    const int biases[] = {INT_MIN, INT_MIN+17, -46341, -7, -1, 0, 1, 17, 46341, INT_MAX-17, INT_MAX};
    const int rounds[] = {FE_TONEAREST, FE_DOWNWARD, FE_UPWARD, FE_TOWARDZERO};
    const double inputs[] = {1e100, 1.0, -1e100, 1.0, 0.0, -0.0, 1e-300, -1e-300};
    long page = sysconf(_SC_PAGESIZE);
    if (page < 1024) return 2;
    char *mem = mmap(0, (size_t)page * 3, PROT_NONE, MAP_PRIVATE|MAP_ANONYMOUS, -1, 0);
    if (mem == MAP_FAILED || mprotect(mem + page, (size_t)page, PROT_READ|PROT_WRITE)) return 2;
    unsigned checks = 0;
    for (unsigned mode = 0; mode < 4; ++mode) {
        if (fesetround(rounds[mode])) return 2;
        for (int n = -3; n <= 65; ++n) {
            /* For n<=0 the pointer denotes an inaccessible guard page. */
            double *v = (double *)(mem + 2 * page) - (n > 0 ? n : 0);
            for (int i = 0; i < n; ++i) v[i] = inputs[(unsigned)i % 8];
            for (unsigned b = 0; b < sizeof biases / sizeof biases[0]; ++b) {
                for (unsigned k = 0; k < 3; ++k) {
                    feclearexcept(FE_ALL_EXCEPT);
                    double expected = ref[k](v, n, biases[b]);
                    int ref_flags = fetestexcept(FE_ALL_EXCEPT);
                    feclearexcept(FE_ALL_EXCEPT);
                    double actual = got[k](v, n, biases[b]);
                    int actual_flags = fetestexcept(FE_ALL_EXCEPT);
                    if (!same(expected, actual) || ref_flags != actual_flags) {
                        fprintf(stderr, "k=%u n=%d bias=%d rounding=%u: %.17g != %.17g flags=%x/%x\n",
                                k, n, biases[b], mode, expected, actual, ref_flags, actual_flags);
                        return 1;
                    }
                    ++checks;
                }
            }
        }
    }
    fesetround(FE_TONEAREST);
    if (munmap(mem, (size_t)page * 3)) return 2;
    printf("strict packed: %u output/exception/guard-page comparisons PASS\n", checks);
    return 0;
}
