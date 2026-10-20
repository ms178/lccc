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
        int ref = __builtin_popcountll(v);
        if (popcnt_u64_while(v) != ref) {
            printf("FAIL u64 i=%u\n", i);
            return 1;
        }
        if (popcnt_guarded_dowhile(v) != ref) {
            printf("FAIL dowhile i=%u\n", i);
            return 2;
        }
        uint32_t v32 = (uint32_t)v;
        unsigned ref32 = (unsigned)__builtin_popcount(v32) + 11u;
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
    printf("ok %ld\n", sum);
    return 0;
}
