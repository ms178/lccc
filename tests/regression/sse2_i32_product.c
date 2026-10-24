/* Packed low-dword products must be legal without SSE4.1. Overflow is
   intentional (-fwrapv); every positive-length input ends at a guard page.
   Odd lengths also exercise unaligned vector loads and all scalar tails. */
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>
#include <unistd.h>

__attribute__((noinline)) void product_lanes(int *restrict out, const int *restrict a, const int *restrict b) {
    out[0] = a[0] * b[0];
    out[1] = a[1] * b[1];
    out[2] = a[2] * b[2];
    out[3] = a[3] * b[3];
}

__attribute__((noinline)) void product_map(int *restrict out, const int *restrict a, const int *restrict b, int n) {
    for (int i = 0; i < n; ++i) out[i] = a[i] * b[i];
}

static uint64_t lane_hash(const int *a, int n) {
    uint64_t h = 1469598103934665603ull;
    for (int i = 0; i < n; ++i) h = (h ^ (uint32_t)a[i]) * 1099511628211ull;
    return h;
}

__attribute__((noinline)) int product_dot(const int *a, const int *b, int n) {
    int sum = 0;
    for (int i = 0; i < n; ++i) sum += a[i] * b[i];
    return sum;
}

__attribute__((noinline)) int product_dual(const int *a, const int *b, const int *c, int n) {
    int ab = 0, ac = 0;
    for (int i = 0; i < n; ++i) {
        ab += a[i] * b[i];
        ac += a[i] * c[i];
    }
    return ab ^ ac;
}

/* The FP argument lives across the packed multiply, whose SSE2 lowering uses
   xmm2 as scratch. The scalar allocator must reserve that register too. */
__attribute__((noinline)) double product_seeded(const int *a, const int *b, int n, double seed) {
    int sum = 0;
    for (int i = 0; i < n; ++i) sum += a[i] * b[i];
    return seed + (double)sum;
}

int main(void) {
    long page = sysconf(_SC_PAGESIZE);
    if (page < 1024) return 2;
    char *maps[4];
    for (int k = 0; k < 4; ++k) {
        maps[k] = mmap(0, (unsigned long)page * 2, PROT_READ | PROT_WRITE,
                       MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        if (maps[k] == MAP_FAILED || mprotect(maps[k] + page, (unsigned long)page, PROT_NONE)) return 3;
    }
    const uint32_t edge[] = {0, 1, 0xffffffffu, 0x80000000u, 0x7fffffffu,
                            65535, 65536, 0xffff0001u, 0xaaaaaaaa, 0x55555555};
    uint32_t state = 0x179b3d21;
    for (int trial = 0; trial < 8; ++trial) {
        for (int n = -3; n <= 97; ++n) {
            int *a[3];
            for (int k = 0; k < 3; ++k) {
                a[k] = (int *)(maps[k] + page) - (n > 0 ? n : 0);
                for (int i = 0; i < n; ++i) {
                    state = state * 1664525u + 1013904223u;
                    a[k][i] = (int)(trial < 2 ? edge[(i + k * 3 + trial) % 10] : state);
                }
            }
            int *out = (int *)(maps[3] + page) - (n > 0 ? n : 0);
            product_map(out, a[0], a[1], n);
            uint64_t mapped = lane_hash(out, n);
            uint64_t lanes = 0;
            if (n >= 4) {
                product_lanes(out, a[0], a[1]);
                lanes = lane_hash(out, 4);
            }
            union { double d; uint64_t u; } result;
            result.d = product_seeded(a[0], a[1], n, trial & 1 ? -0.25 : 0.5);
            printf("%d %d %08x %08x %08x %016llx %016llx %016llx\n", trial, n,
                   (unsigned)product_dot(a[0], a[1], n),
                   (unsigned)product_dot(a[0], a[0], n),
                   (unsigned)product_dual(a[0], a[1], a[2], n),
                   (unsigned long long)result.u,
                   (unsigned long long)mapped, (unsigned long long)lanes);
        }
    }
    for (int k = 0; k < 4; ++k)
        if (munmap(maps[k], (unsigned long)page * 2)) return 4;
    return 0;
}
