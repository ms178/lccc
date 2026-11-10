/* Range-gated same-width Cast fold: 32-bit induction-variable proof.
 *
 * `loop_count` has a U32 induction variable and a U32 bound.  A conservative
 * backend used to widen both to U64, adding one prologue instruction and
 * 64-bit compare/increment operations.  The range proof now keeps the loop in
 * its native U32 domain: `i` starts at zero, increases by one only while
 * `i < n`, and stops at `n <= UINT32_MAX`, so the increment never wraps.
 *
 * The harness compiles this fixture with LCCC, GCC 16.2, Clang 23.1.5 and
 * (in the authenticated hosted lane) ICX 2026.2, then compares a checksum
 * over every `n` in 0..4096 and a deterministic 4096-byte bitmap.  It also
 * compiles with `CCC_SIMPLIFY_SKIP=cast_sign_range` to prove the improvement
 * belongs to that fold.  The assembly and final-IR censuses are the
 * architecture-independent proof; the runtime checksum is an end-to-end
 * correctness check, not a substitute for the IR proof.
 */
#include <stdio.h>
#include <stdint.h>

typedef uint32_t u32;
typedef uint64_t u64;

void hit(u32 x) { (void)x; }

__attribute__((noinline)) u32 loop_count(const unsigned char *b, u32 n) {
    u32 s = 0;
    for (u32 i = 0; i < n; i++)
        s += (b[i >> 3] >> (i & 7)) & 1;
    return s;
}

static unsigned char bits[4096];

int main(int argc, char **argv) {
    volatile unsigned seed = (unsigned)argc + 1u;
    for (unsigned i = 0; i < sizeof(bits); i++) {
        seed = seed * 1664525u + 1013904223u;
        bits[i] = (unsigned char)(seed >> 24);
    }
    u64 h = 1469598103934665603ULL;
    for (u32 n = 0; n <= 4096; n++) {
        u32 v = loop_count(bits, n);
        h ^= v;
        h *= 1099511628211ULL;
    }
    printf("%016llx\n", (unsigned long long)h);
    (void)argv;
    return 0;
}
