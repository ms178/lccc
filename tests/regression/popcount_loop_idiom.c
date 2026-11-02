/* Regression test for Brian Kernighan population-count loop idiom recognition
 * (`while (x) { x &= x - 1; ++n; }` -> hardware popcount + initial accumulator).
 */
#include <stdint.h>
#include <stdio.h>

__attribute__((noinline))
int popcnt_u64_while(uint64_t x) {
    int n = 0;
    while (x) {
        x &= x - 1;
        n++;
    }
    return n;
}

__attribute__((noinline))
unsigned popcnt_u32_commuted(uint32_t x, unsigned base) {
    while (x) {
        x = (x - 1u) & x;
        base++;
    }
    return base;
}

__attribute__((noinline))
int popcnt_guarded_dowhile(uint64_t x) {
    int n = 0;
    if (x) {
        do {
            x &= x - 1;
            ++n;
        } while (x);
    }
    return n;
}

__attribute__((noinline))
long popcnt_array_sum(const uint64_t *a, int len) {
    long total = 0;
    for (int i = 0; i < len; ++i) {
        uint64_t v = a[i];
        int c = 0;
        while (v) {
            v &= v - 1;
            ++c;
        }
        total += c;
    }
    return total;
}

/* Fixed-bit reference: no bit-clear recurrence or popcount builtin. */
static unsigned reference_count(uint64_t x) {
    unsigned n = 0;
    for (unsigned i = 0; i < 64; ++i)
        n += (unsigned)((x >> i) & 1u);
    return n;
}

__attribute__((noinline))
static unsigned popcnt_u8(uint8_t x) {
    unsigned n = 0;
    while (x) { x &= x - 1; ++n; }
    return n;
}

__attribute__((noinline))
static unsigned popcnt_u16(uint16_t x) {
    unsigned n = 0;
    while (x) { x &= x - 1; ++n; }
    return n;
}

/* The wide condition is deliberately NOT the normalized next recurrence.
 * At x=-128, wide=-256 and next x=0: the original loop takes two trips. */
__attribute__((noinline))
static unsigned wide_test(signed char x) {
    unsigned n = 0;
    int wide;
    if (!x) return 0;
    do {
        wide = (int)x & ((int)x - 1);
        x = (signed char)(unsigned char)wide;
        ++n;
    } while (wide);
    return n;
}

__attribute__((noinline))
static unsigned wide_test_i16(int16_t x) {
    unsigned n = 0;
    int32_t wide;
    if (!x) return 0;
    do {
        wide = (int32_t)x & ((int32_t)x - 1);
        x = (int16_t)(uint16_t)wide;
        ++n;
    } while (wide);
    return n;
}

__attribute__((noinline))
static unsigned wide_test_i32(int32_t x) {
    unsigned n = 0;
    int64_t wide;
    if (!x) return 0;
    do {
        wide = (int64_t)x & ((int64_t)x - 1);
        x = (int32_t)(uint32_t)wide;
        ++n;
    } while (wide);
    return n;
}

int main(void) {
    static const uint64_t samples[] = {
        0ULL,
        1ULL,
        2ULL,
        3ULL,
        0x5555555555555555ULL,
        0xAAAAAAAAAAAAAAAAULL,
        0x8000000000000000ULL,
        0x7FFFFFFFFFFFFFFFULL,
        0xFFFFFFFFFFFFFFFFULL,
        0x123456789ABCDEF0ULL,
    };
    long sum = 0;
    for (unsigned i = 0; i < sizeof(samples) / sizeof(samples[0]); ++i) {
        uint64_t v = samples[i];
        int ref = (int)reference_count(v);
        if (popcnt_u64_while(v) != ref) {
            printf("FAIL u64 i=%u\n", i);
            return 1;
        }
        if (popcnt_guarded_dowhile(v) != ref) {
            printf("FAIL dowhile i=%u\n", i);
            return 2;
        }
        uint32_t v32 = (uint32_t)v;
        unsigned ref32 = reference_count(v32) + 11u;
        if (popcnt_u32_commuted(v32, 11u) != ref32) {
            printf("FAIL u32 i=%u\n", i);
            return 3;
        }
        sum += ref;
    }
    if (popcnt_array_sum(samples, (int)(sizeof(samples) / sizeof(samples[0]))) != sum) {
        printf("FAIL sum\n");
        return 4;
    }
    for (unsigned i = 0; i < 65536; ++i) {
        unsigned ref = reference_count(i);
        if (popcnt_u16((uint16_t)i) != ref) return 5;
        if (i < 256 && popcnt_u8((uint8_t)i) != ref) return 6;
        if (popcnt_u32_commuted(i, UINT32_MAX - 3u) != ref + (UINT32_MAX - 3u)) return 7;
    }
    for (int i = -128; i <= 127; ++i) {
        unsigned ref = reference_count((uint8_t)i) + (unsigned)(i < 0);
        if (wide_test((signed char)i) != ref) return 8;
    }
    for (int i = -32768; i <= 32767; ++i) {
        unsigned ref = reference_count((uint16_t)i) + (unsigned)(i < 0);
        if (wide_test_i16((int16_t)i) != ref) return 9;
    }
    static const int32_t signed_samples[] = {
        0, 1, -1, INT32_MIN, INT32_MAX, INT32_MIN / 2, -123456789
    };
    for (unsigned i = 0; i < sizeof(signed_samples) / sizeof(signed_samples[0]); ++i) {
        int32_t x = signed_samples[i];
        unsigned ref = reference_count((uint32_t)x) + (unsigned)(x < 0);
        if (wide_test_i32(x) != ref) return 10;
    }
    printf("ok %ld\n", sum);
    return 0;
}
