/* Runtime contract for __seg_gs array loops that an optimizer may vectorize.
 *
 * A loop over a segment array must keep every access %gs-relative at every
 * optimization level. The vectorizer once rebuilt the reduction over a
 * __seg_gs array with generic vector loads, so -O3 read the wrong memory and
 * returned a wrong sum. Each loop here is checked against the same arithmetic
 * over a generic array, and the segment results are read back through the
 * mirror (&object + GS_DISP), which a correct %gs store writes.
 */
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <unistd.h>

#define ARCH_SET_GS 0x1001
#define GS_DISP ((uintptr_t)1 << 32)
#define MIR(g) ((void *)((uintptr_t)(g) + GS_DISP))

#define N 256

long __seg_gs vec_a[N];
long __seg_gs vec_b[N];
extern long vec_a_g[N] __asm__("vec_a");
extern long vec_b_g[N] __asm__("vec_b");

static int failures;

#define CHECK(cond, name)                                  \
    do {                                                   \
        if (cond) printf("ok   %s\n", name);               \
        else { printf("FAIL %s\n", name); failures++; }    \
    } while (0)

static void map_mirror(const void *g, size_t n) {
    uintptr_t lo = (uintptr_t)g + GS_DISP;
    uintptr_t end = lo + n;
    for (uintptr_t page = lo & ~(uintptr_t)4095; page < end; page += 4096) {
        void *m = mmap((void *)page, 4096, PROT_READ | PROT_WRITE,
                       MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE, -1, 0);
        if (m == MAP_FAILED && errno != EEXIST) {
            perror("mmap");
            _Exit(2);
        }
    }
}

/* Reductions, maps and dot products over segment arrays. Kept out of line so
 * the loops are optimized as loops, not folded into the caller. */
__attribute__((noinline)) static long sum_seg(void) {
    long s = 0;
    for (int i = 0; i < N; i++) s += vec_a[i];
    return s;
}

__attribute__((noinline)) static long dot_seg(void) {
    long s = 0;
    for (int i = 0; i < N; i++) s += vec_a[i] * vec_b[i];
    return s;
}

__attribute__((noinline)) static void map_seg(void) {
    for (int i = 0; i < N; i++) vec_b[i] = vec_a[i] * 2 + 1;
}

__attribute__((noinline)) static void fill_seg(long k) {
    for (int i = 0; i < N; i++) vec_a[i] = (long)i * 3 + k;
}

static long gen_a[N], gen_b[N];

int main(void) {
    map_mirror(vec_a_g, sizeof(long) * N);
    map_mirror(vec_b_g, sizeof(long) * N);
    if (syscall(SYS_arch_prctl, ARCH_SET_GS, (void *)GS_DISP) != 0) {
        perror("arch_prctl(ARCH_SET_GS)");
        return 2;
    }

    /* Seed through ordinary pointers at the mirror. */
    long *ma = (long *)MIR(vec_a_g);
    long *mb = (long *)MIR(vec_b_g);
    for (int i = 0; i < N; i++) {
        ma[i] = (long)i * 7 + 1;
        mb[i] = 0;
        gen_a[i] = (long)i * 7 + 1;
        gen_b[i] = 0;
    }

    long want_sum = 0;
    for (int i = 0; i < N; i++) want_sum += gen_a[i];
    CHECK(sum_seg() == want_sum, "sum_reduction");

    long want_dot = 0;
    for (int i = 0; i < N; i++) {
        gen_b[i] = gen_a[i] * 2 + 1;
        want_dot += gen_a[i] * gen_b[i];
    }
    map_seg();
    int map_ok = 1;
    for (int i = 0; i < N; i++) map_ok &= mb[i] == gen_b[i];
    CHECK(map_ok, "map_store");
    CHECK(dot_seg() == want_dot, "dot_product");

    fill_seg(5);
    int fill_ok = 1;
    for (int i = 0; i < N; i++) fill_ok &= ma[i] == (long)i * 3 + 5;
    CHECK(fill_ok, "fill_store");

    printf("%s (%d failure%s)\n", failures ? "FAILED" : "PASSED", failures,
           failures == 1 ? "" : "s");
    return failures ? 1 : 0;
}
