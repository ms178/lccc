/* Executed with -fwrapv. Dynamic signed bounds must not imply unsigned order.
 * Volatile narrow references prevent the IV widening being tested from proving
 * its own oracle. All paths are bounded; no enormous object is manufactured. */
#include <stdint.h>
#include <stdio.h>
#define NI __attribute__((noinline))
#define MIX() do { h = h * UINT64_C(131) + (uint32_t)p[i & 7]; h ^= (uint32_t)(i * 4); } while (0)
static volatile int32_t bound_input;
static NI uint64_t ordinary(int32_t n, const int32_t *p) {
    uint64_t h = 7; int32_t i;
    for (i = 0; i < n; ++i) { MIX(); }
    return h ^ (uint32_t)i;
}
static NI uint64_t near_max(int32_t n, const int32_t *p) {
    uint64_t h = 7; int32_t i;
    for (i = INT32_MAX - 3; n > i; ++i) { MIX(); }
    return h ^ (uint32_t)i;
}
static NI uint64_t inclusive_wrap(int32_t n, const int32_t *p) {
    uint64_t h = 7; int32_t i; unsigned samples = 0;
    for (i = INT32_MAX - 3; i <= n; ++i) {
        MIX(); if (++samples == 8) break;
    }
    return h ^ (uint32_t)i;
}
static NI uint64_t stride_wrap(int32_t n, const int32_t *p) {
    uint64_t h = 7; int32_t i; unsigned samples = 0;
    for (i = INT32_MAX - 1; !(i >= n); i += 2) {
        MIX(); if (++samples == 8) break;
    }
    return h ^ (uint32_t)i;
}
static NI uint64_t negative_seed(int32_t n, const int32_t *p) {
    uint64_t h = 7; int32_t i;
    for (i = -2; i < n; ++i) { MIX(); }
    return h ^ (uint32_t)i;
}
/* Positive activation witnesses: unlike MIX, no overflowing derived multiply
 * forces the whole widening closure to remain narrow. */
static NI uint64_t clean_runtime(int32_t n, const int32_t *p) {
    uint64_t h = 7; int32_t i;
    for (i = 0; i < n; ++i) h = h * UINT64_C(131) + (uint32_t)p[i & 7];
    return h ^ (uint32_t)i;
}
static NI uint64_t clean_near_max(int32_t n, const int32_t *p) {
    uint64_t h = 7; int32_t i;
    for (i = INT32_MAX - 3; n > i; ++i) h = h * UINT64_C(131) + (uint32_t)p[i & 7];
    return h ^ (uint32_t)i;
}
static NI uint64_t reference(unsigned kind, int32_t n, const int32_t *p) {
    volatile int32_t narrow = (kind == 0 || kind == 5) ? 0 : kind == 4 ? -2 :
        kind == 3 ? INT32_MAX - 1 : INT32_MAX - 3;
    uint64_t h = 7; unsigned samples = 0;
    for (;;) {
        int32_t i = narrow;
        if (kind == 2 ? i > n : i >= n) break;
        /* Unsigned multiplication expresses the original narrow ring directly. */
        h = h * UINT64_C(131) + (uint32_t)p[(uint32_t)i & 7u];
        if (kind < 5) h ^= (uint32_t)i * UINT32_C(4);
        ++samples;
        if ((kind == 2 || kind == 3) && samples == 8) break;
        if (samples > 1024) return UINT64_MAX;
        narrow = (int32_t)((uint32_t)i + (kind == 3 ? UINT32_C(2) : UINT32_C(1)));
    }
    return h ^ (uint32_t)narrow;
}
/* Unknown signed seeds exercise the same strict-header theorem without
 * assuming nonnegativity. Narrow product / inclusive / strided variants must
 * retain modular behavior, even when the address expression itself is safe. */
static volatile int32_t seed_input;
static NI uint64_t clean_runtime_seed(int32_t seed, int32_t n, const int32_t *p) {
    uint64_t h = 7; int32_t i;
    for (i = seed; i < n; ++i) h = h * UINT64_C(131) + (uint32_t)p[i & 7];
    return h ^ (uint32_t)i;
}
static NI uint64_t reversed_runtime_seed(int32_t seed, int32_t n, const int32_t *p) {
    uint64_t h = 7; int32_t i;
    for (i = seed; !(n <= i); ++i) h = h * UINT64_C(131) + (uint32_t)p[i & 7];
    return h ^ (uint32_t)i;
}
static NI uint64_t product_runtime_seed(int32_t seed, int32_t n, const int32_t *p) {
    uint64_t h = 7; int32_t i;
    for (i = seed; i < n; ++i) { MIX(); }
    return h ^ (uint32_t)i;
}
static NI uint64_t stride_runtime_seed(int32_t seed, int32_t n, const int32_t *p) {
    uint64_t h = 7; int32_t i; unsigned samples = 0;
    for (i = seed; i < n; i += 2) {
        MIX(); if (++samples == 8) break;
    }
    return h ^ (uint32_t)i;
}
static NI uint64_t inclusive_runtime_seed(int32_t seed, int32_t n, const int32_t *p) {
    uint64_t h = 7; int32_t i; unsigned samples = 0;
    for (i = seed; i <= n; ++i) {
        MIX(); if (++samples == 8) break;
    }
    return h ^ (uint32_t)i;
}
static NI uint64_t direct_runtime_seed(int32_t seed, int32_t n, const int32_t *p) {
    uint64_t h = 7; int32_t i;
    for (i = seed; i < n; ++i) h = h * UINT64_C(131) + (uint32_t)p[i];
    return h ^ (uint32_t)i;
}
static NI uint64_t seed_reference(unsigned kind, int32_t seed, int32_t n, const int32_t *p) {
    volatile int32_t narrow = seed;
    uint64_t h = 7; unsigned samples = 0;
    for (;;) {
        int32_t i = narrow;
        if (kind == 4 ? i > n : i >= n) break;
        h = h * UINT64_C(131) + (uint32_t)p[kind == 5 ? i : (int32_t)((uint32_t)i & 7u)];
        if (kind >= 2 && kind < 5) h ^= (uint32_t)i * UINT32_C(4);
        if ((kind == 3 || kind == 4) && ++samples == 8) break;
        narrow = (int32_t)((uint32_t)i + (kind == 3 ? UINT32_C(2) : UINT32_C(1)));
    }
    return h ^ (uint32_t)narrow;
}
int main(void) {
    static const int32_t bounds[] = { INT32_MIN, -1, 0, 1, 2, 7, 8, 9, 31, 32, 33, 127,
        INT32_MAX-4, INT32_MAX-3, INT32_MAX-2, INT32_MAX-1, INT32_MAX };
    static const int32_t data[8] = {0, -1, 17, INT32_MAX, INT32_MIN, 3, 5, 99};
    static uint64_t (*const kernels[])(int32_t, const int32_t *) = {
        ordinary, near_max, inclusive_wrap, stride_wrap, negative_seed, clean_runtime, clean_near_max
    };
    unsigned cases = 0;
    for (unsigned kind = 0; kind < 7; ++kind) {
        for (unsigned b = 0; b < sizeof(bounds)/sizeof(bounds[0]); ++b) {
            if ((kind == 0 || kind == 4 || kind == 5) && bounds[b] > 127) continue;
            bound_input = bounds[b];
            uint64_t expected = reference(kind, bound_input, data);
            uint64_t actual = kernels[kind](bound_input, data);
            if (actual != expected) {
                printf("runtime IV mismatch: %u %d %llx %llx\n", kind, bounds[b],
                    (unsigned long long)actual, (unsigned long long)expected);
                return 1;
            }
            ++cases;
        }
    }
    static const int32_t endpoints[] = { INT32_MIN, INT32_MIN+1, INT32_MIN+4,
        -17, -3, -1, 0, 1, 3, 17, INT32_MAX-4, INT32_MAX-1, INT32_MAX };
    static uint64_t (*const seed_kernels[])(int32_t, int32_t, const int32_t *) = {
        clean_runtime_seed, reversed_runtime_seed, product_runtime_seed,
        stride_runtime_seed, inclusive_runtime_seed
    };
    for (unsigned kind = 0; kind < 5; ++kind) {
        for (unsigned a = 0; a < sizeof(endpoints)/sizeof(endpoints[0]); ++a) {
            for (unsigned b = 0; b < sizeof(endpoints)/sizeof(endpoints[0]); ++b) {
                if ((int64_t)endpoints[b] - endpoints[a] > 128) continue;
                seed_input = endpoints[a]; bound_input = endpoints[b];
                uint64_t expected = seed_reference(kind, seed_input, bound_input, data);
                uint64_t actual = seed_kernels[kind](seed_input, bound_input, data);
                if (actual != expected) {
                    printf("seed IV mismatch: %u %d %d\n", kind, endpoints[a], endpoints[b]);
                    return 1;
                }
                ++cases;
            }
        }
    }
    /* Direct signed addresses, with real storage on both sides of the base. */
    static const int32_t padded[17] = {1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17};
    for (int32_t a = -8; a <= 8; ++a) {
        for (int32_t b = -8; b <= 9; ++b) {
            seed_input = a; bound_input = b;
            uint64_t expected = seed_reference(5, seed_input, bound_input, padded + 8);
            if (direct_runtime_seed(seed_input, bound_input, padded + 8) != expected) {
                printf("direct seed IV mismatch: %d %d\n", a, b);
                return 1;
            }
            ++cases;
        }
    }
    printf("runtime signed IV: PASS %u\n", cases);
    return cases != 1000;
}
