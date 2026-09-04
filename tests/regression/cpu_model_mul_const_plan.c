/* CPU tuning model — multiply-by-constant strength reduction
 * (`X86Tune::mul_const_plan`, consumed by isel).  Every constant whose plan
 * fits the P-core budget (2 steps) or the Gracemont budget (4 steps) is
 * exercised at 32/64 bits, signed and unsigned, with values that overflow
 * the narrower type so the wrap-around semantics of the LEA/SHL/ADD/SUB/NEG
 * chain are checked against the reference multiply done by the C library
 * of the oracle compiler.  The multiplicand is passed through `volatile`
 * so the constant folder cannot remove the multiply. */
#include <stdio.h>
#include <stdint.h>

#define K_LIST(X) \
    X(2) X(3) X(4) X(5) X(6) X(7) X(8) X(9) X(10) X(11) X(12) X(13) X(14) X(15) X(16) \
    X(17) X(18) X(19) X(20) X(24) X(25) X(27) X(28) X(31) X(33) X(34) X(36) X(40) X(45) \
    X(48) X(63) X(65) X(72) X(80) X(81) X(96) X(100) X(127) X(129) X(255) X(257) \
    X(1000) X(1023) X(1025) X(4096) X(65535) X(65537) X(1000000) X(2147483647)

#define DEF64(k) \
    __attribute__((noinline)) int64_t  m64_##k(int64_t x)  { return x * (int64_t)k; } \
    __attribute__((noinline)) int64_t  n64_##k(int64_t x)  { return x * -(int64_t)k; } \
    __attribute__((noinline)) uint64_t u64_##k(uint64_t x) { return x * (uint64_t)k; } \
    __attribute__((noinline)) int32_t  m32_##k(int32_t x)  { return x * (int32_t)k; } \
    __attribute__((noinline)) int32_t  n32_##k(int32_t x)  { return x * -(int32_t)k; } \
    __attribute__((noinline)) uint32_t u32_##k(uint32_t x) { return x * (uint32_t)k; }
K_LIST(DEF64)

static uint64_t h = 1469598103934665603ULL;
static void mix(uint64_t v) { h = (h ^ v) * 1099511628211ULL; }

int main(void) {
    static const int64_t xs[] = { 0, 1, -1, 2, 3, 7, -7, 1000, -1000, 123456789,
        -123456789, 2147483647, -2147483648LL, 4294967295LL, 4294967296LL,
        0x7fffffffffffffffLL, (int64_t)0x8000000000000000ULL, 0x5555555555555555LL };
    for (unsigned i = 0; i < sizeof xs / sizeof xs[0]; i++) {
        volatile int64_t vx = xs[i];
        int64_t x = vx;
#define RUN(k) \
        mix((uint64_t)m64_##k(x)); mix((uint64_t)n64_##k(x)); mix(u64_##k((uint64_t)x)); \
        mix((uint32_t)m32_##k((int32_t)x)); mix((uint32_t)n32_##k((int32_t)x)); mix(u32_##k((uint32_t)x));
        K_LIST(RUN)
    }
    printf("%llu\n", (unsigned long long)h);
    printf("%lld %lld %u %d\n", (long long)m64_10(7), (long long)n64_7(-3), u32_17(0xFFFFFFFFu), m32_12(-5));
    return 0;
}
