#include <limits.h>
#include <stdint.h>
#include <stdio.h>

struct cell { uint64_t x, y, z; };
static struct cell cells[65];

__attribute__((noinline))
static uint64_t up(const struct cell *p, int first, int limit) {
    uint64_t sum = 7;
    for (int i = first; i < limit; ++i) {
        sum += p[i].x;
        sum ^= p[i].y;
    }
    return sum;
}

__attribute__((noinline))
static uint64_t down(const struct cell *p, int first, int limit) {
    uint64_t sum = 7;
    for (int i = first; i > limit; --i) {
        sum += p[i].x;
        sum ^= p[i].y;
    }
    return sum;
}

/* A pointer-width reference counter has no narrow signed-wrap transition. */
__attribute__((noinline))
static uint64_t reference(const struct cell *p, int first, int limit, int step) {
    uint64_t sum = 7;
    long long i = first;
    while (step > 0 ? i < (long long)limit : i > (long long)limit) {
        sum += p[i].x;
        sum ^= p[i].y;
        i += step;
    }
    return sum;
}

/* These operations intentionally wrap under the fixture's -fwrapv contract.
   A proof about i's strict unit step must NOT be reused for i+2 or i*24. */
__attribute__((noinline))
static uint64_t narrow_offsets(const struct cell *p, int first, int limit, int multiply) {
    uint64_t sum = 0;
    for (int i = first; i < limit; ++i) {
        int offset = multiply ? i * 24 : i + 2;
        sum += p[(unsigned)offset & 31u].z;
    }
    return sum;
}

static uint64_t offset_reference(const struct cell *p, int first, int limit, int multiply) {
    uint64_t sum = 0;
    for (long long i = first; i < (long long)limit; ++i) {
        uint32_t offset = multiply ? (uint32_t)i * 24u : (uint32_t)i + 2u;
        sum += p[offset & 31u].z;
    }
    return sum;
}

int main(void) {
    for (unsigned i = 0; i < 65; ++i) {
        cells[i].x = (uint64_t)(i + 1) * UINT64_C(0x9e3779b185ebca87);
        cells[i].y = (uint64_t)(i + 3) * UINT64_C(0xc2b2ae3d27d4eb4f);
        cells[i].z = (uint64_t)(i + 5) * UINT64_C(0x165667b19e3779f9);
    }
    const struct cell *center = cells + 32;
    unsigned checks = 0;
    for (int first = -16; first <= 16; ++first) {
        for (int limit = -16; limit <= 16; ++limit) {
            if (up(center, first, limit) != reference(center, first, limit, 1)) return 1;
            if (down(center, first, limit) != reference(center, first, limit, -1)) return 2;
            checks += 2;
        }
    }
    const int empty[][2] = {
        {INT_MIN, INT_MIN}, {INT_MAX, INT_MAX}, {INT_MAX, INT_MIN},
    };
    for (unsigned i = 0; i < sizeof(empty) / sizeof(empty[0]); ++i) {
        if (up(center, empty[i][0], empty[i][1]) != 7) return 3;
        if (down(center, empty[i][1], empty[i][0]) != 7) return 4;
        checks += 2;
    }
    const int starts[] = {INT_MIN, INT_MIN + 8, -9, -2, 0, 9, INT_MAX / 24 - 4, INT_MAX - 8};
    for (unsigned n = 0; n < sizeof(starts) / sizeof(starts[0]); ++n) {
        for (int length = 0; length <= 8; ++length) {
            for (int multiply = 0; multiply <= 1; ++multiply) {
                int first = starts[n], limit = first + length;
                if (narrow_offsets(cells, first, limit, multiply)
                    != offset_reference(cells, first, limit, multiply)) return 5;
                ++checks;
            }
        }
    }
    printf("ivsr_signed_unit: PASS (%u checks)\n", checks);
    return 0;
}
