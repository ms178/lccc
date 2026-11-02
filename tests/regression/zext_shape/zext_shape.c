/* Two zero-extension shapes whose emitted form regressed once already.
 *
 * 1. `bitcast_float` widens the BITS of a float to u64. The emitter lowers the
 *    bit cast as `movq %xmmN, %rX` (upper half is FP payload garbage) and the
 *    widening as `movl %eX, %eX`; `movd %xmmN, %eX` is the same architectural
 *    state in one instruction. The pre-fusion form must not come back.
 *
 * 2. `wide_divide` widens a 32-bit division remainder to u64. `divl` writes
 *    BOTH %eax and %edx 32 bits wide, so each register's upper half is already
 *    zero and the self-zext the emitter adds is a no-op.
 *
 * Every value is printed so the program is also a run-time check: the shape
 * gate and the correctness gate read the same binary.
 */
#include <stdio.h>

#define N 64
static float pool[N];
static unsigned acc;

/* 1: float bits -> u64, mixed into a 64-bit hash. */
static unsigned long long bitcast_float(int i) {
    union {
        float f;
        unsigned u;
    } bits = {pool[i]};
    unsigned long long h = 0x9e3779b97f4a7c15ULL;
    h ^= (unsigned long long)bits.u;
    h *= 0x100000001b3ULL;
    return h;
}

/* 2: 32-bit division, remainder widened to u64. */
static unsigned long long wide_divide(unsigned x, unsigned y) {
    unsigned r = x % y;
    return 0x123456789abcdefULL + (unsigned long long)r;
}

int main(void) {
    for (int i = 0; i < N; i++) {
        unsigned s = (unsigned)i * 2654435761u;
        pool[i] = (float)(s & 0x7fffffff) / 7.0f;
    }
    unsigned long long h = 0;
    for (int i = 0; i < N; i++)
        h ^= bitcast_float(i);
    acc = (unsigned)h;
    printf("hash %016llx\n", h);

    unsigned long long d = 0;
    for (unsigned i = 1; i < 40; i++)
        d ^= wide_divide(1000003u * i + 7u, (i & 7u) + 3u);
    printf("div  %016llx\n", d);
    printf("acc  %08x\n", acc);
    return 0;
}
