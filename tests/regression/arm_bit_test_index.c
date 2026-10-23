/* BitTest `(x >> i) & 1` with constant indices across the whole immediate
 * range.
 *
 * AArch64 lowered an immediate index >= the operand width as
 * `lsr w0, w1, #40`, which no assembler accepts: an undefined C shift in
 * code that never runs must still compile (GCC accepts it with a warning).
 * The IR defines the index modulo the width (x86 BT, AArch64 LSRV and
 * RISC-V SRL/SRLW all do), so every immediate is now one `ubfx` of bit
 * `i mod W` -- which also turns the in-range edges 31 and 63, formerly
 * `lsr; and`, into one instruction.
 *
 * scripts/check_arm_codegen_assembles.sh compiles and assembles every
 * arm_*.c for AArch64 (and byte-compares against GNU as); the regression
 * runner executes this file natively. Only defined (in-range) indices are
 * executed; the out-of-range functions are emitted, never called. */
typedef unsigned long uint64_t;
extern int printf(const char *, ...);

#define BIT32(name, k)                                                       \
    __attribute__((noinline)) int name(int x) { return (x >> (k)) & 1; }      \
    __attribute__((noinline)) int name##_u(unsigned x) {                      \
        return (int)((x >> (k)) & 1u);                                        \
    }
#define BIT64(name, k)                                                       \
    __attribute__((noinline)) long name(long x) { return (x >> (k)) & 1; }    \
    __attribute__((noinline)) long name##_u(uint64_t x) {                     \
        return (long)((x >> (k)) & 1u);                                       \
    }

BIT32(b32_0, 0)
BIT32(b32_7, 7)
BIT32(b32_30, 30)
BIT32(b32_31, 31)
BIT64(b64_0, 0)
BIT64(b64_33, 33)
BIT64(b64_62, 62)
BIT64(b64_63, 63)

/* Never called: undefined shifts that must still compile and assemble. */
#pragma GCC diagnostic ignored "-Wshift-count-overflow"
BIT32(b32_32_never, 32)
BIT32(b32_40_never, 40)
BIT64(b64_64_never, 64)
BIT64(b64_70_never, 70)

__attribute__((noinline)) int b32_var(int x, int i) { return (x >> i) & 1; }
__attribute__((noinline)) long b64_var(long x, int i) { return (x >> i) & 1; }

static volatile int never_call;

int main(void) {
    static const unsigned pats32[] = {0u, 1u, 0x80u, 0x40000000u, 0x80000000u,
                                      0xffffffffu, 0x7fffffffu, 0xa5a5a5a5u};
    static const uint64_t pats64[] = {0ul,
                                      1ul,
                                      0x200000000ul,
                                      0x4000000000000000ul,
                                      0x8000000000000000ul,
                                      ~0ul,
                                      0x7ffffffffffffffful,
                                      0xa5a5a5a55a5a5a5aul};
    unsigned long sum = 0;
    for (unsigned n = 0; n < sizeof pats32 / sizeof pats32[0]; n++) {
        unsigned p = pats32[n];
        int s = (int)p;
        int bits = b32_0(s) | b32_7(s) << 1 | b32_30(s) << 2 | b32_31(s) << 3 |
                   b32_0_u(p) << 4 | b32_7_u(p) << 5 | b32_30_u(p) << 6 |
                   b32_31_u(p) << 7;
        printf("32 %08x -> %02x var %d%d%d\n", p, bits, b32_var(s, 0),
               b32_var(s, 30), b32_var(s, 31));
        sum = sum * 31u + (unsigned)bits;
    }
    for (unsigned n = 0; n < sizeof pats64 / sizeof pats64[0]; n++) {
        uint64_t p = pats64[n];
        long s = (long)p;
        long bits = b64_0(s) | b64_33(s) << 1 | b64_62(s) << 2 |
                    b64_63(s) << 3 | b64_0_u(p) << 4 | b64_33_u(p) << 5 |
                    b64_62_u(p) << 6 | b64_63_u(p) << 7;
        printf("64 %016lx -> %02lx var %ld%ld%ld\n", p, bits, b64_var(s, 0),
               b64_var(s, 62), b64_var(s, 63));
        sum = sum * 31u + (unsigned long)bits;
    }
    if (never_call) {
        sum += (unsigned long)(b32_32_never(1) + b32_40_never(1) +
                               b32_32_never_u(1) + b32_40_never_u(1)) +
               (unsigned long)(b64_64_never(1) + b64_70_never(1) +
                               b64_64_never_u(1) + b64_70_never_u(1));
    }
    printf("sum %lu\n", sum);
    return 0;
}
