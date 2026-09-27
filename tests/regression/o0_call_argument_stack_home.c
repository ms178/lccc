/*
 * CC-O0CALL: register allocation is disabled at -O0, so accumulator-address loads must not treat a cache entry as proof of a
 * pointer definition when a numeric id has multiple definitions.  `pick` makes the first argument's
 * evaluation use a pointer base; the second argument then independently
 * reloads the current pointer value for its dereference.
 */
#include <stdint.h>

static volatile uint64_t left = UINT64_C(0x112233445566778);
static volatile uint64_t right = -UINT64_C(0x102030405060708);

__attribute__((noinline)) static uint64_t *pick(uint64_t *p) {
    return p;
}

__attribute__((noinline)) static uint64_t combine(uint64_t a, uint64_t b) {
    return (a * 17) ^ (b + 9);
}

static uint64_t exercise(unsigned rounds) {
    uint64_t *p = (uint64_t *)&left;
    uint64_t total = 0;

    for (unsigned i = 0; i < rounds; ++i) {
        p = (i & 1u) ? (uint64_t *)&right : (uint64_t *)&left;
        total ^= combine(*pick(p), *p);
        total += combine(*p, *pick(p));
    }
    return total;
}

int main(void) {
    return exercise(9) != UINT64_C(0xf5ee02be80049ee2);
}
