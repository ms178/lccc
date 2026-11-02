/* Exact division after modular multiplication. Unsigned multiplication avoids
 * signed-overflow UB; conversions are the supported targets' two's complement.
 * Non-inlined volatile reference prevents circular proof by the same fold. */
#include <stdint.h>
#include <stdio.h>
#define FN __attribute__((noinline))
FN int32_t half32(uint32_t x) { return (int32_t)(x * (x + 1u)) / 2; }
FN int64_t half64(uint64_t x) { return (int64_t)(x * (x + 1ull)) / 2; }
FN int32_t odd32(uint32_t x) { return (int32_t)(x * (x + 2u)) / 2; }
FN int64_t xor64(uint64_t x) { return (int64_t)(x * (x ^ 3ull)) / 2; }
FN int64_t different(uint64_t x, uint64_t y) { return (int64_t)(x * (y + 1ull)) / 2; }
FN int64_t ref64(int64_t x) { volatile int64_t p=x; return p / 2; }
int main(void) {
    uint64_t x=0x123456789abcdef0ull;
    for (unsigned i=0;i<10000;i++) {
        x=x*6364136223846793005ull+1;
        if (i<64) x=1ull<<i;
        uint32_t a=(uint32_t)x;
        if (half32(a)!=ref64((int32_t)(a*(a+1u))) ||
            half64(x)!=ref64((int64_t)(x*(x+1ull))) ||
            odd32(a)!=ref64((int32_t)(a*(a+2u))) ||
            xor64(x)!=ref64((int64_t)(x*(x^3ull))) ||
            different(x,x^0x10001ull)!=ref64((int64_t)(x*((x^0x10001ull)+1)))) return 1;
    }
    puts("PASS: exact product halves, wrapped negative inputs and odd/unrelated near-misses");
    return 0;
}
