/*
 * clz/ctz/popcount must respect the TARGET ISA contract, not the build
 * host's. LZCNT/TZCNT/POPCNT are x86-64-v2/v3 features; the baseline
 * (v1) fallbacks must produce identical results to the native paths,
 * including the defined-at-zero semantics the IR guarantees
 * (Clz(0)/Ctz(0) == width for 32/64-bit operands via constant folding).
 *
 * Differential vs GCC over edge patterns; every value printed was chosen
 * to expose the BSR-vs-LZCNT decode difference on non-ABM CPUs:
 * bit-31-set words (bsr gives 31, lzcnt gives 0) and the zero word
 * (BSR/BSF leave the destination undefined; the fallback must fix it).
 */
#include <stdio.h>

static int clz32(unsigned x) { return __builtin_clz(x); }
static int ctz32(unsigned x) { return __builtin_ctz(x); }
static int clz64(unsigned long long x) { return __builtin_clzll(x); }
static int ctz64(unsigned long long x) { return __builtin_ctzll(x); }
static int pc32(unsigned x) { return __builtin_popcount(x); }
static int pc64(unsigned long long x) { return __builtin_popcountll(x); }

static unsigned long long mix(unsigned long long h, unsigned long long v)
{
        return (h ^ v) * 0x9e3779b97f4a7c15ULL + 0x2545f4914f6cdd1dULL;
}

int main(void)
{
        unsigned long long h = 1469598103934665603ULL;
        /* Word patterns that differ between BSR and LZCNT semantics. */
        static const unsigned w32[] = {
                0x00000001u, 0x00000008u, 0x0000ffffu, 0x00010000u,
                0x7fffffffu, 0x80000000u, 0x80000001u, 0xaaaaaaaau,
                0x55555555u, 0xfffffffeu, 0xffffffffu,
        };
        static const unsigned long long w64[] = {
                0x0000000000000001ull, 0x0000000100000000ull, 0x00000000ffffffffull,
                0xffffffff00000000ull, 0x8000000000000000ull, 0x7fffffffffffffffull,
                0xaaaaaaaaaaaaaaaaull, 0x5555555555555555ull, 0xfffffffffffffffeull,
                0xffffffffffffffffull,
        };

        for (unsigned i = 0; i < sizeof(w32) / sizeof(w32[0]); i++) {
                unsigned x = w32[i];
                h = mix(h, (unsigned long)clz32(x));
                h = mix(h, (unsigned long)ctz32(x));
                h = mix(h, (unsigned)pc32(x));
                /* rotate one bit to vary low bits across patterns */
                x = (x << 1) | (x >> 31);
                h = mix(h, (unsigned long)clz32(x));
                h = mix(h, (unsigned long)ctz32(x));
                h = mix(h, (unsigned)pc32(x));
        }
        for (unsigned i = 0; i < sizeof(w64) / sizeof(w64[0]); i++) {
                unsigned long long x = w64[i];
                h = mix(h, (unsigned long long)clz64(x));
                h = mix(h, (unsigned long long)ctz64(x));
                h = mix(h, (unsigned)pc64(x));
                }
        /* NOTE: zero inputs are deliberately NOT hashed — gcc treats
         * __builtin_clz(0)/ctz(0) as UB and constant-folds them to values
         * that differ from lccc's defined Clz(0)/Ctz(0) == width contract.
         * The defined-zero semantics are pinned by the compiler's own
         * constant folder and the emission checks, not by this differential
         * (which must stay meaningful under gcc). */

        printf("%016llx\n", h);
        return 0;
}
