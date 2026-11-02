/* Guarded Clz/Ctz ternaries — `x ? __builtin_clz(x) : width`. The IR defines
 * Clz(0)/Ctz(0) == operand width, so these selects are redundant and must
 * collapse onto the intrinsic (no duplicated zero fix-up / dead cmov tail).
 * Differential (lccc vs gcc); the codegen gate lives in
 * check_clz_zero_guard_fold.sh. */
#include <stdio.h>
#include <stdint.h>

volatile unsigned sink;

unsigned lz32(unsigned x) { return x ? __builtin_clz(x) : 32; }
unsigned lz32b(unsigned x) { unsigned t = x ? __builtin_clz(x) : 32; return t; }
int lz32s(int x) { return x ? __builtin_clz(x) : 32; }
unsigned lz64(uint64_t x) { return x ? __builtin_clzll(x) : 64; }
unsigned tz32(unsigned x) { return x ? __builtin_ctz(x) : 32; }
unsigned tz64(uint64_t x) { return x ? __builtin_ctzll(x) : 64; }
unsigned lz16(unsigned short x) { return x ? __builtin_clz((unsigned)x) : 16; }
unsigned tz16(unsigned short x) { return x ? __builtin_ctz((unsigned)x) : 16; }

int main(void) {
    unsigned s = 0x12345678u;
    unsigned long long v = 0ull;
    for (unsigned i = 0; i < 32; i++) {
        v ^= lz32(s << i);
        v ^= lz32(1u << i);
        v ^= tz32(1u << i);
    }
    for (uint64_t i = 0; i < 64; i++) {
        v ^= lz64(1ull << i);
        v ^= lz64((1ull << i) - 1);
        v ^= tz64(1ull << i);
        v ^= tz64(0x8000000000000000ull >> (i & 63));
    }
    v ^= lz32s(-1) + lz32s(0) + lz32s(0x7fffffff);
    v ^= lz32b(0x100) + lz32b(0);
    v ^= lz16(0x8000) + lz16(0) + lz16(1) + lz16(0x00ff);
    v ^= tz16(0x8000) + tz16(0) + tz16(1);
    v ^= lz32(0) ^ tz32(0) ^ lz64(0) ^ tz64(0);
    sink = (unsigned)(v ^ (v >> 32));
    printf("%08x\n", sink);
    return 0;
}
