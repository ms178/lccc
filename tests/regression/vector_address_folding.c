/* Packed field addresses: globals, ordinary and runtime-overaligned stack
 * objects, variable trip counts, tails, and untouched guard fields. The
 * volatile reference cannot take the vector address paths being tested. */
#include <stdint.h>
#include <stdio.h>
#define NI __attribute__((noinline))
struct cell { double x, y, vx, vy; uint64_t guard; };
static struct cell global_cells[18];

static NI void advance(struct cell *p, unsigned n) {
    for (unsigned i = 0; i < n; ++i) {
        p[i].x += p[i].vx * 0.5;
        p[i].y += p[i].vy * 0.5;
        p[i].vx += 0.25;
        p[i].vy -= 0.5;
    }
}
static NI void reference(volatile struct cell *p, unsigned n) {
    for (unsigned i = 0; i < n; ++i) {
        p[i].x += p[i].vx * 0.5;
        p[i].y += p[i].vy * 0.5;
        p[i].vx += 0.25;
        p[i].vy -= 0.5;
    }
}
static NI int probe(uint64_t seed, unsigned n, uint64_t *checksum) {
    struct cell ordinary[18];
    _Alignas(64) struct cell aligned[18];
    volatile struct cell expected[18];
    for (unsigned i = 0; i < 18; ++i) {
        double x = (double)((seed + i) & 255u);
        double y = (double)((seed >> 8) & 127u) - i;
        double vx = (double)(i + 1), vy = -(double)(i + 3);
        uint64_t guard = seed ^ ((uint64_t)i << 40);
        ordinary[i].x = aligned[i].x = global_cells[i].x = expected[i].x = x;
        ordinary[i].y = aligned[i].y = global_cells[i].y = expected[i].y = y;
        ordinary[i].vx = aligned[i].vx = global_cells[i].vx = expected[i].vx = vx;
        ordinary[i].vy = aligned[i].vy = global_cells[i].vy = expected[i].vy = vy;
        ordinary[i].guard = aligned[i].guard = global_cells[i].guard = expected[i].guard = guard;
    }
    for (unsigned round = 0; round < 17; ++round) {
        advance(ordinary, n); advance(aligned, n); advance(global_cells, n);
        reference(expected, n);
    }
    for (unsigned i = 0; i < 18; ++i) {
#define CHECK(field) \
        if (ordinary[i].field != expected[i].field || \
            aligned[i].field != expected[i].field || \
            global_cells[i].field != expected[i].field) return 1;
        CHECK(x) CHECK(y) CHECK(vx) CHECK(vy) CHECK(guard)
#undef CHECK
        *checksum ^= ordinary[i].guard + (uint64_t)(ordinary[i].x + 4096.0);
    }
    return 0;
}
int main(void) {
    uint64_t seed = UINT64_C(0x781aab3398761234), checksum = 0;
    for (unsigned round = 0; round < 32; ++round) {
        seed = seed * UINT64_C(6364136223846793005) + 1;
        for (unsigned n = 0; n <= 17; ++n)
            if (probe(seed, n, &checksum)) return 1;
    }
    printf("vector-address %016llx\n", (unsigned long long)checksum);
    return 0;
}
