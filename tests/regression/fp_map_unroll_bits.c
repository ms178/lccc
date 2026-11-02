/* Guard endpoints, strict multiply/add rounding, signed zeros, subnormals,
   and sequential alias semantics. No fast-math or contraction is permitted. */
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>
#include <unistd.h>
#ifdef FP_MAP_DOUBLE
typedef double real;
typedef uint64_t word;
static const real xs[] = {0x1.0000000000001p0, -0.0, 0x1p-1074, 0x1p-1022, 1e200, -1e200, 3.0, -7.0};
static const real ys[] = {0x1.ffffffffffffep-1, 1.0, 1.0, 0.5, -1e200, 1e200, 0.25, -0.5};
#else
typedef float real;
typedef uint32_t word;
static const real xs[] = {0x1.000002p0f, -0.0f, 0x1p-149f, 0x1p-126f, 1e30f, -1e30f, 3.0f, -7.0f};
static const real ys[] = {0x1.fffffcp-1f, 1.0f, 1.0f, 0.5f, -1e30f, 1e30f, 0.25f, -0.5f};
#endif
static const real zs[] = {-1.0, -0.0, 0.0, 0.0, 1.0, -1.0, -2.0, 4.0};
__attribute__((noinline))
real checked_fp_map(real *d, const real *x, const real *y, const real *z, int n, real seed) {
    for (int i = 0; i < n; ++i) d[i] = x[i] * y[i] + z[i];
    /* Keep a scalar FP argument live across both packed copies. */
    return seed * (real)0.5 + (real)0.125;
}
int main(void) {
    long page = sysconf(_SC_PAGESIZE);
    if (page < (long)(256 * sizeof(real))) return 2;
    char *maps[4];
    real *base[4];
    for (int k = 0; k < 4; ++k) {
        maps[k] = mmap(0, (unsigned long)page * 2, PROT_READ | PROT_WRITE,
                       MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        if (maps[k] == MAP_FAILED || mprotect(maps[k] + page, (unsigned long)page, PROT_NONE)) return 3;
        base[k] = (real *)(maps[k] + page) - 256;
    }
    const int offsets[] = {-17, -1, 0, 1, 17};
    for (int mode = -1; mode < 5; ++mode) {
        for (int n = -3; n <= 97; ++n) {
            for (int i = 0; i < 256; ++i) {
                base[0][i] = xs[i & 7];
                base[1][i] = ys[i & 7];
                base[2][i] = zs[i & 7];
                base[3][i] = (real)17;
            }
            int start = mode < 0 ? 256 - (n > 0 ? n : 0) : 128;
            real *d = mode < 0 ? base[3] + start : base[0] + start + offsets[mode];
            union { real value; word bits; } live;
            live.value = checked_fp_map(d, base[0] + start, base[1] + start,
                                        base[2] + start, n, (real)n * (real)0.25);
            uint64_t hash = 1469598103934665603ull ^ live.bits;
            for (int k = 0; k < 4; ++k) {
                for (int i = 0; i < 256; ++i) {
                    union { real value; word bits; } u;
                    u.value = base[k][i];
                    hash = (hash ^ u.bits) * 1099511628211ull;
                }
            }
            printf("%d %d %016llx\n", mode, n, (unsigned long long)hash);
        }
    }
    for (int k = 0; k < 4; ++k)
        if (munmap(maps[k], (unsigned long)page * 2)) return 4;
    return 0;
}
