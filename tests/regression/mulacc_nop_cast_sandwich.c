/* The canon walk may only cross SINGLE-USE no-op I64<->U64 casts (audit
 * refinement): multi-use or value-changing casts must break the chain.
 * These shapes sandwich the head/tail between aliases, splits and joins
 * that stress that rule; every fused or rejected variant must print the
 * same values as GCC. */
#include <stdio.h>
#include <stdint.h>

static uint64_t digest;

static void sink(uint64_t v) { digest = digest * 31 + v; }

/* (u64)(s64)x: canonical no-op; use it TWICE (multi-use) — the chain
 * through x must not be built (single-use rule) but the value must be. */
static uint64_t multi_use(int32_t x) {
    uint64_t a = (uint64_t)(int64_t)x * 10 + 5;
    uint64_t b = (uint64_t)(int64_t)x * 7;
    return a + b;
}

/* split: chain result consumed by both a 32-bit and a 64-bit use */
static uint64_t split_use(uint32_t seed) {
    uint64_t h = seed;
    h = h * 6364136223846793005ull + 1442695040888963407ull;
    uint32_t lo = (uint32_t)h;
    uint64_t out = (h >> 32) ^ lo;
    sink(out);
    return out;
}

/* nested: a chain whose addend is itself a chain result */
static uint64_t nested(uint32_t seed) {
    uint64_t inner = (uint64_t)seed * 2654435761u + 12345u;
    uint64_t outer = inner * 0xFFFFFFFFull + (uint64_t)(uint32_t)inner;
    return outer;
}

/* self-referential wraparound */
static uint64_t wrap(uint32_t seed) {
    uint64_t r = seed;
    for (int i = 0; i < 16; i++)
        r = r * 0xFFFFFFFFull + r;
    return r;
}

int main(void) {
    volatile int32_t x = -1000000;
    volatile uint32_t s = 0xDEADBEEFu;
    printf("A=%016llx\n", (unsigned long long)multi_use(x));
    for (int i = 0; i < 4; i++) printf("B%d=%016llx\n", i, (unsigned long long)split_use(s + (uint32_t)i));
    printf("C=%016llx\n", (unsigned long long)nested(s));
    printf("D=%016llx\n", (unsigned long long)wrap(s));
    printf("DIG=%016llx\n", (unsigned long long)digest);
    return 0;
}
