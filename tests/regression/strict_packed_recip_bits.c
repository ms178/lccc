/* All integer overflow here is intentional and defined by -fwrapv.
   Compare stdout bitwise with independent strict-FP GCC/Clang executables.
   Every positive-length array ends exactly at a protected page boundary. */
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>
#include <unistd.h>

__attribute__((noinline)) double packed_sum(const double *a, int n, int bias, double seed) {
    double sum = seed;
    for (int j = 0; j < n; ++j) {
        int t = j + bias;
        int denominator = (((t*t + 5*t - 11) ^ (j << 1)) >> 1) | 1;
        sum += (1.0 / (double)denominator) * a[j];
    }
    return sum;
}

int main(void) {
    long page = sysconf(_SC_PAGESIZE);
    if (page <= 0) return 2;
    char *p = mmap(0, (unsigned long)page*2, PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED || mprotect(p+page, (unsigned long)page, PROT_NONE)) return 3;
    const int biases[] = {-2147483647-1, -50000, -9, 0, 17, 50000, 2147483647};
    const double seeds[] = {-0.0, 0.0, 1e-300, -1e200};
    const double values[] = {-0.0, 0.0, 1.0, -1.0, 1e-300, -1e-300, 1e200, -1e200};
    for (int n = -3; n <= 41; ++n) {
        double *a = (double *)(p + page) - (n > 0 ? n : 0);
        for (int j = 0; j < n; ++j) a[j] = values[j & 7];
        for (int b = 0; b < 7; ++b) {
            for (int s = 0; s < 4; ++s) {
                union { double d; uint64_t u; } bits;
                bits.d = packed_sum(a, n, biases[b], seeds[s]);
                printf("%d %d %d %016llx\n", n, b, s, (unsigned long long)bits.u);
            }
        }
    }
    return munmap(p, (unsigned long)page*2) != 0;
}
