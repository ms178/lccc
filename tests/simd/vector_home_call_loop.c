/* Reproducer for the vector-home-across-call gap recorded in
 * engineering/SIMD-IMPLEMENTATION-AUDIT-2026-11-08.md, section 9c.10.12.
 *
 * `f1` is deliberately one instruction and `noinline`, so any timing
 * difference between compilers is attributable to the CALLER, not the
 * callee.  GCC 16.2 keeps both 256-bit operands in registers across the call
 * (interprocedural clobber analysis) and emits `call; sub; jne`.  LCCC spills
 * and reloads both operands through over-aligned stack homes every
 * iteration, reaching them indirectly via `lea`+`and`, which also defeats
 * every slot-based peephole.
 *
 * Build:  <cc> -O2 -mavx2 -mfma vector_home_call_loop.c -o /tmp/vh
 * Run:    /tmp/vh            (prints cycles per ITERS calls, best of ROUNDS)
 *
 * The accumulating value deliberately overflows to +inf; that is identical
 * on every compiler and keeps the loop from being deleted, so the measurement
 * compares call/spill overhead rather than arithmetic.  The printed sink is
 * not a correctness oracle -- this file measures, it does not assert.
 *
 * No headers beyond immintrin/stdio/stdint, no undefined behaviour: unsigned
 * wraparound for the timestamp arithmetic, and the vector load reads a
 * fully initialised eight-element array.
 */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>

typedef float v8 __attribute__((vector_size(32)));

#define ITERS 200000
#define ROUNDS 20

v8 f1(v8 a, v8 b);

__attribute__((noinline)) v8 f1(v8 a, v8 b) { return a * b + a; }

static inline uint64_t ticks(void) {
    uint32_t lo, hi;
    __asm__ __volatile__("lfence; rdtsc" : "=a"(lo), "=d"(hi));
    return ((uint64_t)hi << 32) | lo;
}

int main(void) {
    const float seed[8] = {1.0f, 2.0f, 3.0f, 4.0f, 5.0f, 6.0f, 7.0f, 8.0f};
    v8 a = _mm256_loadu_ps(seed);
    const v8 b = a;
    uint64_t best = UINT64_MAX;

    for (int r = 0; r < ROUNDS; r++) {
        const uint64_t t0 = ticks();
        for (int i = 0; i < ITERS; i++) {
            a = f1(a, b);
        }
        const uint64_t elapsed = ticks() - t0;
        if (elapsed < best) {
            best = elapsed;
        }
    }
    printf("%llu cycles / %d calls  sink=%g\n", (unsigned long long)best, ITERS,
           (double)a[0]);
    return 0;
}
