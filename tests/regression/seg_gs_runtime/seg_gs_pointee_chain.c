/* Runtime, self-checking contract for a segment qualifier written after an
 * earlier `*` of a declarator (the pointee of the outer pointer is in %gs).
 *
 *   long *__seg_gs *pp;   pp is ordinary memory. *pp is a `long *` object that
 *                         lives in %gs. **pp is the long it points at.
 *
 * The GS base is set to `area`, so the value of `pp` is an offset into `area`
 * where the segment pointer object sits. Each check compares the observed
 * value with the same value read through ordinary memory. GCC 14.2 -O2 is the
 * oracle (the output must match it line for line).
 */
#include <stdio.h>
#include <sys/syscall.h>
#include <unistd.h>

#define ARCH_SET_GS 0x1001
#define AREA 4096

static unsigned char area[AREA] __attribute__((aligned(64)));
static long target[8];
static int failures;

#define CHECK(cond, name)                 \
    do {                                  \
        if (cond) {                       \
            printf("ok %s\n", name);      \
        } else {                          \
            printf("FAIL %s\n", name);    \
            failures++;                   \
        }                                 \
    } while (0)

/* Two levels: the first star's qualifier puts the pointer it builds in %gs. */
static long *__seg_gs *pp;

/* Three levels: the qualifier after star 1 puts the star-1 pointer in %gs, and
 * the qualifier after star 2 puts the star-2 pointer in %gs. The object itself
 * (star 3, adjacent to the name) is ordinary memory. */
static long *__seg_gs *__seg_gs *ppp;

__attribute__((noinline)) static long read_two(long *__seg_gs *q) { return **q; }
__attribute__((noinline)) static void write_two(long *__seg_gs *q, long v) { **q = v; }
__attribute__((noinline)) static long read_plus_one(void) { return **pp + 1; }

int main(void) {
    if (syscall(SYS_arch_prctl, ARCH_SET_GS, area) != 0) {
        perror("arch_prctl");
        return 2;
    }
    setvbuf(stdout, NULL, _IONBF, 0);

    /* The segment object at GS:0x100 holds a generic pointer to target[2]. */
    long **slot = (long **)(area + 0x100);
    target[2] = 41;
    *slot = &target[2];

    pp = (long *__seg_gs *)0x100;
    CHECK(**pp == 41, "two_level_read");
    **pp = 42;
    CHECK(target[2] == 42, "two_level_write");
    CHECK(read_two(pp) == 42, "two_level_arg_read");
    write_two(pp, 43);
    CHECK(target[2] == 43, "two_level_arg_write");
    CHECK(read_plus_one() == 44, "two_level_global_read");

    /* Three levels. GS:0x140 is a star-2 object: it holds the GS offset of the
     * star-1 object. GS:0x148 is that star-1 object: a generic pointer. */
    *(unsigned long *)(area + 0x140) = 0x148;
    *(long **)(area + 0x148) = &target[5];
    target[5] = 77;
    ppp = (long *__seg_gs *__seg_gs *)0x140;
    CHECK(**ppp == &target[5], "three_level_pointer_value");
    CHECK(***ppp == 77, "three_level_read");
    ***ppp = 78;
    CHECK(target[5] == 78, "three_level_write");

    if (failures == 0) {
        printf("PASSED (0 failures)\n");
        return 0;
    }
    printf("FAILED (%d failure%s)\n", failures, failures == 1 ? "" : "s");
    return 1;
}
