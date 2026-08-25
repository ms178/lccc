/* Regression: a Select whose constant arm is staged into %rcx
 * (`movq $0, %rcx; cmovcc %rcx, %rax`) must invalidate the emitter's
 * register cache. Before the fix, `reg_cache.sec` still claimed %rcx held
 * the previously staged constant K; the next `operand_to_rcx(K)` skipped
 * the reload and `(h ^ v) * K` multiplied the clobbered register (zero) —
 * bitops_builtins returned garbage whenever the copy-propagation peephole
 * failed to rescue the consumers. This test recreates the shape directly:
 * a big-constant multiply interleaved with a branchless select that uses
 * %rcx. Differential vs GCC. */
#include <stdio.h>

static unsigned long long xs[64];

static unsigned long long mix(unsigned long long h, unsigned long long v) {
    return (h ^ v) * 0x9e3779b97f4a7c15ULL + 0xdeadbeefULL;
}

int main(void) {
    unsigned long long h = 1469598103934665603ULL;
    for (int i = 0; i < 64; i++)
        xs[i] = (unsigned long long)(i * 2654435761u) ^ 0x9e3779b9ULL;

    unsigned long long acc = h;
    for (int i = 0; i < 64; i++) {
        unsigned long long v = xs[i];
        /* The select's constant false-arm stages through rcx; the multiply
         * consumes the register-homed big constant afterwards. */
        unsigned long long sel = (v & 1) ? v : 0ULL;
        acc = mix(acc, v);
        acc ^= sel * 3ULL;
        /* Second select with the constant as the TRUE arm (cmov direction
         * flipped) to cover both staging orders. */
        unsigned long long sel2 = (v & 2) ? 0x1234567890abcdefULL : v;
        acc = mix(acc, sel2);
        acc += (acc >> 31);
    }
    printf("%llu\n", (unsigned long long)acc);
    return 0;
}
