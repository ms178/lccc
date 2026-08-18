/* Mix-hash chain miscompile regression (x86-64 MachInst + mul-add fusion).
 *
 * `mix()` is the classic 64-bit integer hash step:
 *
 *     (h ^ v) * 0x9e3779b97f4a7c15ULL + 0xdeadbeefULL
 *
 * Its IR is `Xor; Mul; Add`. The Mul+Add pair is lowered through the
 * mul-add fusion (default accumulator path), while the Xor was buffered in
 * the MachInst pipeline. The fused emission read the Xor's result register
 * BEFORE the buffered `xorq` was flushed, so `h` was multiplied un-xored and
 * the xor landed after the imul — a silent wrong hash.
 *
 * This test feeds every 64-bit edge value (0, 1, 7, 2^64-1, 2^63, and a
 * 61-bit value) through a loop-carried mix chain and checks the final hash
 * against a GCC-compiled reference. It must stay exact on all backends.
 */
#include <stdio.h>
#include <stdint.h>

static uint64_t mix(uint64_t h, uint64_t v)
{
    return (h ^ v) * 0x9e3779b97f4a7c15ULL + 0xdeadbeefULL;
}

int main(void)
{
    uint64_t h = 1469598103934665603ULL;
    static const uint64_t ys[] = {
        0ULL, 1ULL, 7ULL, 0xffffffffffffffffULL,
        0x8000000000000000ULL, 1234567890123456789ULL,
    };
    unsigned n;
    for (n = 0; n < sizeof(ys) / sizeof(ys[0]); n++)
        h = mix(h, ys[n]);
    printf("%llu\n", (unsigned long long)h);
    return 0;
}
