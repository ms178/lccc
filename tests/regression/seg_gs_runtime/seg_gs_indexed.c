/* Runtime, self-checking contract for indexed __seg_gs accesses: the array
 * element forms `obj[i]`, `obj[i + k]`, and `p[i]` through a parameter base,
 * for every integer width and pointer element, in loads, stores, immediate
 * stores (with truncation), and read-modify-write. The GS base is set to
 * GS_DISP and each object is mirrored at (&object + GS_DISP); a correct %gs
 * access reads and writes the mirror, and every expected value comes from an
 * explicit formula, not from the code under test.
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
#define MIR(g) ((void *)((char *)(g) + GS_DISP))
#define N 16

long __seg_gs ix_l[N];
int __seg_gs ix_i[N];
unsigned char __seg_gs ix_b[N];
short __seg_gs ix_s[N];
unsigned int __seg_gs ix_u[N];
void *__seg_gs ix_p[N];

/* Generic-address aliases of the same objects, for the mirror and mmap calls. */
extern long g_l[N] __asm__("ix_l");
extern int g_i[N] __asm__("ix_i");
extern unsigned char g_b[N] __asm__("ix_b");
extern short g_s[N] __asm__("ix_s");
extern unsigned int g_u[N] __asm__("ix_u");
extern void *g_p[N] __asm__("ix_p");

static int failures;

#define CHECK(cond, name)                                  \
    do {                                                   \
        if (cond) printf("ok   %s\n", name);               \
        else { printf("FAIL %s\n", name); failures++; }    \
    } while (0)

static void map_mirror(void *g, size_t n) {
    uintptr_t lo = (uintptr_t)g + GS_DISP;
    uintptr_t page = lo & ~(uintptr_t)4095;
    size_t len = ((lo + n - page) + 4095) & ~(size_t)4095;
    void *m = mmap((void *)page, len, PROT_READ | PROT_WRITE,
                   MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE, -1, 0);
    if (m != (void *)page && errno != EEXIST) {
        perror("mmap");
        _Exit(2);
    }
}

/* Loads and stores at scale 8, 4, 2 and 1, with a variable index. */
__attribute__((noinline)) static void fill_l(long n) {
    for (long i = 0; i < n; i++) ix_l[i] = 7 * i + 1;
}
__attribute__((noinline)) static long sum_l(long n) {
    long s = 0;
    for (long i = 0; i < n; i++) s += ix_l[i];
    return s;
}
__attribute__((noinline)) static void fill_i(long n) {
    for (long i = 0; i < n; i++) ix_i[i] = (int)(i * 1000003) - 9000000;
}
__attribute__((noinline)) static long sum_i(long n) {  /* sign extension */
    long s = 0;
    for (long i = 0; i < n; i++) s += ix_i[i];
    return s;
}
__attribute__((noinline)) static void fill_s(long n) {
    for (long i = 0; i < n; i++) ix_s[i] = (short)(i * 4001 - 30000);
}
__attribute__((noinline)) static long sum_s(long n) {
    long s = 0;
    for (long i = 0; i < n; i++) s += ix_s[i];
    return s;
}
__attribute__((noinline)) static void fill_b(long n) {  /* truncating store */
    for (long i = 0; i < n; i++) ix_b[i] = (unsigned char)(i * 37 + 200);
}
__attribute__((noinline)) static long sum_b(long n) {  /* zero extension */
    long s = 0;
    for (long i = 0; i < n; i++) s += ix_b[i];
    return s;
}
__attribute__((noinline)) static void fill_u(long n) {
    for (long i = 0; i < n; i++) ix_u[i] = 0xfffffff0u + (unsigned)i;
}
__attribute__((noinline)) static long sum_u(long n) {  /* 32-bit zero extension */
    long s = 0;
    for (long i = 0; i < n; i++) s += (long)ix_u[i];
    return s;
}

/* Immediate stores, including values wider than the element. */
__attribute__((noinline)) static void const_store(long n) {
    for (long i = 0; i < n; i++) {
        ix_l[i] = -5;
        ix_b[i] = (unsigned char)0x1ff;
        ix_s[i] = -2;
    }
}

/* Read-modify-write through one index, and a displaced neighbour read. */
__attribute__((noinline)) static void rmw_l(long n) {
    for (long i = 0; i < n; i++) ix_l[i] += i + 1;
}
__attribute__((noinline)) static void shift_neighbour(long n) {
    for (long i = 0; i + 1 < n; i++) ix_l[i] = ix_l[i + 1] - 1;
}

/* Register base: the object reaches the callee as a parameter. */
__attribute__((noinline)) static void p_fill(long __seg_gs *p, long n) {
    for (long i = 0; i < n; i++) p[i] = 3 * i + 2;
}
__attribute__((noinline)) static long p_sum(long __seg_gs *p, long n) {
    long s = 0;
    for (long i = 0; i < n; i++) s += p[i];
    return s;
}

/* Pointer elements (Ptr width), symbol base. */
__attribute__((noinline)) static void fill_p(long n) {
    for (long i = 0; i < n; i++) ix_p[i] = (void *)(uintptr_t)(i + 1);
}
__attribute__((noinline)) static long sum_p(long n) {
    long s = 0;
    for (long i = 0; i < n; i++) s += (long)(uintptr_t)ix_p[i];
    return s;
}

int main(void) {
    map_mirror(g_l, sizeof g_l);
    map_mirror(g_i, sizeof g_i);
    map_mirror(g_b, sizeof g_b);
    map_mirror(g_s, sizeof g_s);
    map_mirror(g_u, sizeof g_u);
    map_mirror(g_p, sizeof g_p);
    if (syscall(SYS_arch_prctl, ARCH_SET_GS, (void *)GS_DISP) != 0) {
        perror("arch_prctl");
        return 2;
    }

    volatile long *ml = MIR(g_l);
    volatile int *mi = MIR(g_i);
    volatile unsigned char *mb = MIR(g_b);
    volatile short *ms = MIR(g_s);
    volatile unsigned int *mu = MIR(g_u);
    void *volatile *mp = MIR(g_p);

    /* Seed the mirrors through ordinary pointers, then read through %gs. */
    long e;
    for (int i = 0; i < N; i++) ml[i] = 1000 + i * i;
    e = 0;
    for (int i = 0; i < N; i++) e += 1000 + i * i;
    CHECK(sum_l(N) == e, "load_long_sum");

    for (int i = 0; i < N; i++) mi[i] = -i * 3;
    e = 0;
    for (int i = 0; i < N; i++) e += -i * 3;
    CHECK(sum_i(N) == e, "load_int_sign_extend_sum");

    for (int i = 0; i < N; i++) ms[i] = (short)(-300 * i - 1);
    e = 0;
    for (int i = 0; i < N; i++) e += -300 * i - 1;
    CHECK(sum_s(N) == e, "load_short_sign_extend_sum");

    for (int i = 0; i < N; i++) mb[i] = (unsigned char)(250 + i);
    e = 0;
    for (int i = 0; i < N; i++) e += (unsigned char)(250 + i);
    CHECK(sum_b(N) == e, "load_byte_zero_extend_sum");

    for (int i = 0; i < N; i++) mu[i] = 0xf0000000u + (unsigned)i;
    e = 0;
    for (int i = 0; i < N; i++) e += (long)(0xf0000000u + (unsigned)i);
    CHECK(sum_u(N) == e, "load_uint_zero_extend_sum");

    for (int i = 0; i < N; i++) mp[i] = (void *)(uintptr_t)(100 + i);
    e = 0;
    for (int i = 0; i < N; i++) e += 100 + i;
    CHECK(sum_p(N) == e, "load_pointer_sum");

    /* Stores through the segment index, checked on the mirror. */
    fill_l(N);
    int ok = 1;
    for (int i = 0; i < N; i++) ok &= ml[i] == 7 * i + 1;
    CHECK(ok, "store_long_mirror");

    fill_i(N);
    ok = 1;
    for (int i = 0; i < N; i++) ok &= mi[i] == (int)(i * 1000003) - 9000000;
    CHECK(ok, "store_int_mirror");

    fill_s(N);
    ok = 1;
    for (int i = 0; i < N; i++) ok &= ms[i] == (short)(i * 4001 - 30000);
    CHECK(ok, "store_short_mirror");

    fill_b(N);
    ok = 1;
    for (int i = 0; i < N; i++) ok &= mb[i] == (unsigned char)(i * 37 + 200);
    CHECK(ok, "store_byte_truncates_mirror");

    fill_u(N);
    ok = 1;
    for (int i = 0; i < N; i++) ok &= mu[i] == 0xfffffff0u + (unsigned)i;
    CHECK(ok, "store_uint_mirror");

    fill_p(N);
    ok = 1;
    for (int i = 0; i < N; i++) ok &= (uintptr_t)mp[i] == (uintptr_t)(i + 1);
    CHECK(ok, "store_pointer_mirror");

    const_store(N);
    ok = 1;
    for (int i = 0; i < N; i++)
        ok &= ml[i] == -5 && mb[i] == 0xff && ms[i] == -2;
    CHECK(ok, "immediate_store_truncates_mirror");

    /* Fill then read back through the same segment index. */
    fill_l(N);
    e = 0;
    for (int i = 0; i < N; i++) e += 7 * i + 1;
    CHECK(sum_l(N) == e, "store_then_load_long_sum");

    /* Read-modify-write: ix[i] = (7i+1) + (i+1), then a displaced neighbour
     * shift with ix[i] = ix[i+1] - 1, which reads one element ahead. */
    fill_l(N);
    rmw_l(N);
    ok = 1;
    for (int i = 0; i < N; i++) ok &= ml[i] == 7 * i + 1 + i + 1;
    CHECK(ok, "read_modify_write_mirror");
    shift_neighbour(N);
    ok = 1;
    for (int i = 0; i + 1 < N; i++) ok &= ml[i] == 7 * (i + 1) + 1 + (i + 1) + 1 - 1;
    CHECK(ok, "displaced_neighbour_mirror");

    /* Register base through a parameter. */
    p_fill(ix_l, N);
    ok = 1;
    for (int i = 0; i < N; i++) ok &= ml[i] == 3 * i + 2;
    CHECK(ok, "register_base_store_mirror");
    e = 0;
    for (int i = 0; i < N; i++) e += 3 * i + 2;
    CHECK(p_sum(ix_l, N) == e, "register_base_load_sum");

    printf("%s (%d failure%s)\n", failures ? "FAILED" : "PASSED", failures,
           failures == 1 ? "" : "s");
    return failures ? 1 : 0;
}
