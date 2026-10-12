/* Runtime, self-checking contract for aggregate copies to and from __seg_gs
 * objects of every size class: a straight-line copy (<= SEG_COPY_UNROLL_MAX,
 * 512 bytes), the counted-loop copy above it, and copies that happen inside a
 * function whose many live arguments put locals on the %rsp-relative path.
 *
 * Each object's bytes are checked through its mirror (&object + GS_DISP), which
 * a correct %gs copy writes, against a byte pattern. A copy that reads the
 * wrong offset, or a generic copy in place of a segment copy, fails.
 *
 * Covers:
 *   - generic -> segment, segment -> generic and segment -> segment copies of
 *     24, 320, 600, 1500 and 4096 bytes;
 *   - an aggregate returned by value from a segment object;
 *   - a copy inside a function with eight integer arguments (rsp addressing).
 */
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <unistd.h>

#define ARCH_SET_GS 0x1001
#define GS_DISP ((uintptr_t)1 << 32)
#define MIR(g) ((void *)((char *)(g) + GS_DISP))

struct b24 { unsigned char b[24]; };
struct b320 { unsigned char b[320]; };
struct b600 { unsigned char b[600]; };
struct b1500 { unsigned char b[1500]; };
struct b4k { unsigned char b[4096]; };

struct b24 __seg_gs g24;
struct b320 __seg_gs g320;
struct b600 __seg_gs g600;
struct b1500 __seg_gs g1500;
struct b4k __seg_gs g4k;
struct b600 __seg_gs g600_b;
extern struct b24 a24[] __asm__("g24");
extern struct b320 a320[] __asm__("g320");
extern struct b600 a600[] __asm__("g600");
extern struct b1500 a1500[] __asm__("g1500");
extern struct b4k a4k[] __asm__("g4k");
extern struct b600 a600b[] __asm__("g600_b");

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

static unsigned char pat(size_t i, unsigned seed) {
    return (unsigned char)((i * 131u + seed) ^ (i >> 3));
}

static void fill(unsigned char *p, size_t n, unsigned seed) {
    for (size_t i = 0; i < n; i++) p[i] = pat(i, seed);
}

static int matches(const unsigned char *p, size_t n, unsigned seed) {
    for (size_t i = 0; i < n; i++)
        if (p[i] != pat(i, seed)) return 0;
    return 1;
}

/* Generic -> segment, segment -> generic, segment -> segment. */
__attribute__((noinline)) static void to_seg24(const struct b24 *s) { g24 = *s; }
__attribute__((noinline)) static void from_seg24(struct b24 *d) { *d = g24; }
__attribute__((noinline)) static void to_seg320(const struct b320 *s) { g320 = *s; }
__attribute__((noinline)) static void from_seg320(struct b320 *d) { *d = g320; }
__attribute__((noinline)) static void to_seg600(const struct b600 *s) { g600 = *s; }
__attribute__((noinline)) static void from_seg600(struct b600 *d) { *d = g600; }
__attribute__((noinline)) static void to_seg1500(const struct b1500 *s) { g1500 = *s; }
__attribute__((noinline)) static void from_seg1500(struct b1500 *d) { *d = g1500; }
__attribute__((noinline)) static void to_seg4k(const struct b4k *s) { g4k = *s; }
__attribute__((noinline)) static void from_seg4k(struct b4k *d) { *d = g4k; }
__attribute__((noinline)) static void seg_to_seg600(void) { g600 = g600_b; }
__attribute__((noinline)) static struct b320 ret320(void) { return g320; }

/* Eight integer arguments fill the argument registers and spill to the frame,
 * so the locals below are addressed from %rsp in some frame layouts. */
__attribute__((noinline)) static long many_args(long a, long b, long c, long d,
                                                long e, long f, long g, long h,
                                                const struct b600 *src,
                                                const struct b1500 *src2) {
    struct b600 t = *src;
    g600 = t;
    long sum = a + b + c + d + e + f + g + h;
    g1500 = *src2;
    return sum + t.b[0];
}

int main(void) {
    setvbuf(stdout, NULL, _IONBF, 0);
    map_mirror(a24, sizeof(struct b24));
    map_mirror(a320, sizeof(struct b320));
    map_mirror(a600, sizeof(struct b600));
    map_mirror(a1500, sizeof(struct b1500));
    map_mirror(a4k, sizeof(struct b4k));
    map_mirror(a600b, sizeof(struct b600));
    if (syscall(SYS_arch_prctl, ARCH_SET_GS, (void *)GS_DISP) != 0) {
        perror("arch_prctl");
        return 2;
    }

    struct b24 s24;
    struct b320 s320;
    struct b600 s600;
    struct b1500 s1500;
    struct b4k s4k;
    fill(s24.b, sizeof s24.b, 1);
    fill(s320.b, sizeof s320.b, 2);
    fill(s600.b, sizeof s600.b, 3);
    fill(s1500.b, sizeof s1500.b, 4);
    fill(s4k.b, sizeof s4k.b, 5);

    to_seg24(&s24);
    CHECK(matches(((struct b24 *)MIR(a24))->b, 24, 1), "gen_to_seg_24");
    struct b24 r24;
    memset(&r24, 0, sizeof r24);
    from_seg24(&r24);
    CHECK(matches(r24.b, 24, 1), "seg_to_gen_24");

    to_seg320(&s320);
    CHECK(matches(((struct b320 *)MIR(a320))->b, 320, 2), "gen_to_seg_320");
    struct b320 r320 = ret320();
    CHECK(matches(r320.b, 320, 2), "return_seg_320_by_value");
    struct b320 f320;
    from_seg320(&f320);
    CHECK(matches(f320.b, 320, 2), "seg_to_gen_320");

    to_seg600(&s600);
    CHECK(matches(((struct b600 *)MIR(a600))->b, 600, 3), "gen_to_seg_600");
    struct b600 f600;
    from_seg600(&f600);
    CHECK(matches(f600.b, 600, 3), "seg_to_gen_600");

    to_seg1500(&s1500);
    CHECK(matches(((struct b1500 *)MIR(a1500))->b, 1500, 4), "gen_to_seg_1500");
    struct b1500 f1500;
    from_seg1500(&f1500);
    CHECK(matches(f1500.b, 1500, 4), "seg_to_gen_1500");

    to_seg4k(&s4k);
    CHECK(matches(((struct b4k *)MIR(a4k))->b, 4096, 5), "gen_to_seg_4096");
    struct b4k f4k;
    from_seg4k(&f4k);
    CHECK(matches(f4k.b, 4096, 5), "seg_to_gen_4096");

    /* Segment -> segment: g600_b is filled through its mirror, then copied. */
    fill(((struct b600 *)MIR(a600b))->b, 600, 6);
    seg_to_seg600();
    CHECK(matches(((struct b600 *)MIR(a600))->b, 600, 6), "seg_to_seg_600");

    /* Copy inside a function with eight integer arguments. */
    long sum = many_args(1, 2, 3, 4, 5, 6, 7, 8, &s600, &s1500);
    CHECK(sum == 36 + s600.b[0], "many_args_sum");
    CHECK(matches(((struct b600 *)MIR(a600))->b, 600, 3), "many_args_copy_600");
    CHECK(matches(((struct b1500 *)MIR(a1500))->b, 1500, 4), "many_args_copy_1500");

    printf("%s (%d failure%s)\n", failures ? "FAILED" : "PASSED", failures,
           failures == 1 ? "" : "s");
    return failures ? 1 : 0;
}
