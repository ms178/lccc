/* ivwiden_exposed_latch: executed regression for the iv_widen theorem-5'
 * trajectory gates and the exposed-latch intercept.
 *
 * Four case shapes, all with a RUNTIME ring-top seed (0xFFFFFFFF, routed
 * through a volatile so no frontend constant-folds it) inside a mmap'd span
 * of 2^32+3 bytes carrying byte markers just past the 32-bit ring top:
 *
 *   A  rotated do-while whose latch value escapes the loop through
 *      Cast+GEP (`buf[i]` after `while (i++ < n)`).  The exiting pass
 *      evaluates the latch step at the untested seed: narrow wraps to 0,
 *      wide does not.  A pass that widens and lets the escape keep the
 *      wide value reads buf[2^32] = 99 instead of buf[0] = 11
 *      (IVW-RANGE-1, live miscompile vs gcc/clang/icx at -O1..-O3).
 *   B  `buf[(i + 1u) & 63u]` with `while (i++ < n)`: the frontend emits
 *      TWO value-identical Add(phi,1) steps (iv_widen runs ahead of the
 *      merging CSE), so the body read is a use of the latch member.  The
 *      `& 63` masks any divergence away (bit-exact for every unsigned
 *      input), so this loop MUST still widen after the fix — checked by
 *      the unit fixture test_and_const_widens and the debug profile.
 *   C  `buf[i + 2u]` with `while (i++ < 8u)`: a stride-2 twin of the unit
 *      latch step at an exposed latch and runtime seed.  The seed-
 *      inclusive fold [0,max]+2 leaves the type — the plan must decline.
 *      A pass that folds from a strict (phi-guarded) range admits the
 *      twin and reads buf[2^32+2] = 91 instead of buf[1] = 21.
 *   D  guarded counted loop with a hoisted runtime bound and const seed
 *      (phi-guarded strict range, intercept off): must widen; four
 *      iterations x dsink[] = 11 gives 44 under both narrow and wide.
 *
 * Expected (gcc -O0..-O3 oracle): A=11 B=11 C=21 D=44.
 * Pre-fix lccc -O2:               A=99 B=11 C=91 D=44  (exit code 1).
 *
 * Needs a 64-bit address space for the over-read markers; on ILP32 the
 * probe cannot distinguish narrow-wrap from wide-no-wrap, so it prints
 * SKIP and exits 0 (the -m32 arm of the check script pins that path and
 * that the fixture still compiles and runs there).
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>

volatile uint32_t v_seed = 0xFFFFFFFFu; /* ring-top seed, runtime-routed */
volatile uint32_t v_zero = 0u;

static uint8_t *buf; /* 2^32+3-byte span with byte markers */
static uint8_t dsink[256];

/* A: rotated do-while, exit-value escape through Cast(latch)+GEP. */
__attribute__((noinline)) static uint32_t case_a(uint32_t n) {
    uint32_t i = v_seed;
    uint8_t acc = 0;
    do {
        acc = buf[i & 63u]; /* bit-exact body read */
    } while (i++ < n);
    return (uint32_t)acc + (uint32_t)buf[i];
}

/* B: twin Add(phi,1) through And-63 (bit-exact) + unit latch. */
__attribute__((noinline)) static uint32_t case_b(uint32_t n) {
    uint32_t i = v_seed;
    uint8_t acc = 0;
    do {
        acc = buf[(i + 1u) & 63u];
    } while (i++ < n);
    return acc;
}

/* C: twin Add(phi,2) body read + unit latch, const bound. */
__attribute__((noinline)) static uint32_t case_c(void) {
    uint32_t i = v_seed;
    uint8_t acc = 0;
    do {
        acc = buf[i + 2u];
    } while (i++ < 8u);
    return acc;
}

/* D: guarded counted loop, hoisted volatile bound, const seed. */
__attribute__((noinline)) static uint32_t case_d(void) {
    uint32_t bound = v_zero + 12u; /* hoisted runtime bound (opaque) */
    uint32_t acc = 0;
    for (uint32_t j = 8u; j < bound; ++j) {
        acc += dsink[j & 255u];
    }
    return acc;
}

int main(void) {
#if UINTPTR_MAX == 0xffffffffu
    printf("ivwiden_exposed_latch: SKIP (needs a 64-bit address space)\n");
    return 0;
#else
    size_t span = (size_t)0xFFFFFFFFu + 3u;
    buf = mmap(NULL, span, PROT_READ | PROT_WRITE,
               MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
    if (buf == MAP_FAILED) {
        printf("SKIP (mmap refused)\n");
        return 0;
    }
    buf[0] = 11;                             /* A narrow exit read */
    buf[1] = 21;                             /* C narrow twin read */
    buf[(size_t)0xFFFFFFFFu + 1u] = 99;      /* A wide over-read marker */
    buf[(size_t)0xFFFFFFFFu + 2u] = 91;      /* C wide over-read marker */
    for (unsigned k = 0; k < 256u; ++k) dsink[k] = 11u;
    uint32_t n = v_zero;
    uint32_t a = case_a(n);
    uint32_t b = case_b(n);
    uint32_t c = case_c();
    uint32_t d = case_d();
    printf("A=%u\nB=%u\nC=%u\nD=%u\n", a, b, c, d);
    if (a != 11u || b != 11u || c != 21u || d != 44u) {
        printf("ivwiden_exposed_latch: FAIL (want A=11 B=11 C=21 D=44)\n");
        return 1;
    }
    return 0;
#endif
}
