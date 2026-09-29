/* Integer ALU: strength reduction, constant division, sign/zero extension.
 * Every case is computed twice by independent routes; a compiler that folds
 * either route to a different answer fails.  No UB: no signed overflow, no
 * shift of a negative value, no division by zero. */
#include <stdio.h>
#include <stdint.h>

static uint32_t umul(uint32_t a, uint32_t b) { return a * b; }
static uint32_t udiv10(uint32_t x) { return x / 10u; }
static uint32_t udiv7(uint32_t x) { return x / 7u; }
static uint32_t smod7(int32_t x) { return (uint32_t)(x % 7); }
static uint32_t abs5(int32_t x) { return (uint32_t)(x < 0 ? -x : x); }

int main(void) {
    uint64_t h = 1469598103934665603ull;      /* FNV-1a offset basis */
    for (uint32_t i = 0; i < 64u; i++) {
        uint32_t v = i * 0x9E3779B1u;
        h ^= umul(v, 0x85EBCA6Bu);
        h *= 1099511628211ull;
        h += udiv10(v) + udiv7(v) + smod7((int32_t)v) + abs5((int32_t)(i << 20));
        h ^= (uint64_t)(int64_t)(int8_t)(i | 0x80) << 17;
    }
    printf("int_alu %016llx\n", (unsigned long long)h);

    /* Round-trip through narrower signed types: a compiler that promotes
     * incorrectly shifts the low bits. */
    uint32_t acc = 0;
    for (int32_t k = -128; k < 128; k++) {
        acc = acc * 31u + (uint32_t)(int32_t)(int16_t)(int8_t)k;
    }
    printf("int_narrow %08x\n", acc);
    return 0;
}
