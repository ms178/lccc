#include <limits.h>
#include <stdint.h>
#include <stdio.h>

/* Non-SIB stride, two affine siblings, negative indices within one array.
   The unit-level modular model covers the full signed eight-bit domain;
   native pointer witnesses deliberately stay inside their actual C object. */
struct record { uint64_t x, padding[5], y; };
static struct record records[65];

__attribute__((noinline))
static uint64_t ascending(const struct record *p, int first, int limit) {
    uint64_t sum = 11;
    for (int i = first; i < limit; ++i) {
        sum += p[i].x;
        sum ^= p[i + 1].y;
    }
    return sum;
}

__attribute__((noinline))
static uint64_t descending(const struct record *p, int first, int limit) {
    uint64_t sum = 11;
    for (int i = first; i > limit; --i) {
        sum += p[i].x;
        sum ^= p[i - 1].y;
    }
    return sum;
}

__attribute__((noinline))
static uint64_t reference(const struct record *p, int first, int limit, int step) {
    uint64_t sum = 11;
    long long i = first;
    while (step > 0 ? i < (long long)limit : i > (long long)limit) {
        sum += p[i].x;
        sum ^= p[i + step].y;
        i += step;
    }
    return sum;
}

int main(void) {
    for (unsigned i = 0; i < 65; ++i) {
        records[i].x = (uint64_t)(i + 1) * UINT64_C(0x9e3779b185ebca87);
        records[i].y = (uint64_t)(i + 3) * UINT64_C(0xc2b2ae3d27d4eb4f);
    }
    const struct record *center = records + 32;
    unsigned checks = 0;
    for (int first = -16; first <= 16; ++first) {
        for (int limit = -16; limit <= 16; ++limit) {
            if (ascending(center, first, limit) != reference(center, first, limit, 1)) return 1;
            if (descending(center, first, limit) != reference(center, first, limit, -1)) return 2;
            checks += 2;
        }
    }
    const int empty[][2] = {
        {INT_MIN, INT_MIN}, {INT_MAX, INT_MAX}, {INT_MAX, INT_MIN},
    };
    for (unsigned n = 0; n < sizeof(empty) / sizeof(empty[0]); ++n) {
        if (ascending(center, empty[n][0], empty[n][1]) != 11) return 3;
        if (descending(center, empty[n][1], empty[n][0]) != 11) return 4;
        checks += 2;
    }
    printf("ivsr_signed_affine_step: PASS (%u checks)\n", checks);
    return 0;
}
