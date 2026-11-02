/* Parity and population count, checked against an independent model.
 *
 * `__builtin_parity` used to be lowered as `Popcount(x) & 1`: the whole count
 * in a register so that its low bit could be kept.  It is now its own IR
 * operation, so every backend answers the question that was asked -- x86 folds
 * the operand down to a byte and reads the hardware parity flag, the RISCs fold
 * to a nibble and index a 16-bit table -- and `Popcount(x) & 1` in source folds
 * to the same operation.
 *
 * This file checks *values*, not shapes: every case is compared against a
 * reference computed here from the definition (population count by Kernighan's
 * identity, parity as its low bit), so a wrong answer is a failure even if the
 * host's libc, the compiler's own builtins or the oracle agree with each other.
 * `check_parity_and_popcount.sh` runs it at five optimisation levels against
 * GCC and pins the emission; the shapes it does not exercise (the count itself
 * on RISC-V) are covered by the same file because `popcount` is checked too.
 */
#include <stdio.h>

static unsigned long hash = 1469598103934665603ul;
static int failures;

static void mix(unsigned long v) {
    hash = hash * 1099511628211ul + v;
}

static int ref_popcount64(unsigned long x) {
    int n = 0;
    while (x) {
        x &= x - 1; /* Kernighan: one trip per set bit, no SWAR in the model. */
        n++;
    }
    return n;
}

static int ref_popcount32(unsigned x) {
    int n = 0;
    while (x) {
        x &= x - 1;
        n++;
    }
    return n;
}

static void expect(int got, int want, const char *what, unsigned long input) {
    if (got != want) {
        if (failures < 8) {
            printf("  FAIL %s(%lx): got %d want %d\n", what, input, got, want);
        }
        failures++;
    }
    mix((unsigned long)got);
}

int main(void) {
    /* Widths the frontend can reach: the intrinsic's operand type is int for
     * __builtin_parity/popcount and (long) long for the suffixed forms, and a
     * narrow argument is promoted to it.  Both spellings are checked anyway. */
    for (unsigned v = 0; v < 65536u; v++) {
        unsigned short h = (unsigned short)v;
        unsigned char b = (unsigned char)v;
        int pc16 = ref_popcount32(h);
        int pc8 = ref_popcount32(b);
        expect(__builtin_popcount(h), pc16, "popcount(u16)", v);
        expect(__builtin_parity(h), pc16 & 1, "parity(u16)", v);
        expect(__builtin_popcount(b), pc8, "popcount(u8)", v);
        expect(__builtin_parity(b), pc8 & 1, "parity(u8)", v);
        /* The idiom the simplify rule exists for: a parity flag spelled as the
         * low bit of a count.  It must agree with the builtin and the model. */
        expect(__builtin_popcount(h) & 1, pc16 & 1, "popcount&1(u16)", v);
    }

    for (unsigned i = 0; i < 4096u; i++) {
        /* Values that are not compile-time constants: the whole point is the
         * emitted sequence, not the folder. */
        unsigned x = i * 2654435761u + 12345u;
        unsigned long y = (unsigned long)i * 0x9e3779b97f4a7c15ul + 0x0123456789abcdeful;
        int pc32 = ref_popcount32(x);
        int pc64 = ref_popcount64(y);
        expect(__builtin_popcount(x), pc32, "popcount(u32)", x);
        expect(__builtin_parity(x), pc32 & 1, "parity(u32)", x);
        expect(__builtin_popcountl(y), pc64, "popcountl(u64)", y);
        expect(__builtin_parityl(y), pc64 & 1, "parityl(u64)", y);
        expect(__builtin_popcountll(y) & 1, pc64 & 1, "popcountll&1", y);
        /* A parity of a value that is itself the result of arithmetic: this is
         * where a backend that mistook the operand for a constant would show. */
        expect(__builtin_parity(x ^ (unsigned)(y >> 7)), ref_popcount32(x ^ (unsigned)(y >> 7)) & 1,
               "parity(xor)", x);
    }

    /* Edges: zero, all ones, single bits at every position of both widths. */
    expect(__builtin_parity(0u), 0, "parity(0)", 0);
    expect(__builtin_popcount(0u), 0, "popcount(0)", 0);
    expect(__builtin_parity(~0u), 0, "parity(~0)", ~0u);
    expect(__builtin_popcount(~0u), 32, "popcount(~0)", ~0u);
    expect(__builtin_parityl(0ul), 0, "parityl(0)", 0);
    expect(__builtin_popcountl(~0ul), (int)(sizeof(unsigned long) * 8), "popcountl(~0)", ~0ul);
    for (unsigned bit = 0; bit < 32; bit++) {
        unsigned v = 1lu << bit;
        expect(__builtin_parity(v), 1, "parity(1<<n)", v);
        expect(__builtin_popcount(v), 1, "popcount(1<<n)", v);
    }
    {
        unsigned long v = 1ul;
        int bits = (int)(sizeof(unsigned long) * 8);
        for (int bit = 0; bit < bits; bit++) {
            expect(__builtin_parityl(v), 1, "parityl(1<<n)", v);
            expect(__builtin_popcountl(v), 1, "popcountl(1<<n)", v);
            v <<= 1;
        }
    }

    printf("parity_and_popcount: failures=%d hash=%016lx\n", failures, hash);
    return failures != 0;
}
