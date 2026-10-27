#include <limits.h>
#include <stdio.h>

#define NOINLINE __attribute__((noinline))

NOINLINE int i64_low_two_val(long long x) {
    switch (x) {
    case LLONG_MIN: return -7;
    case LLONG_MIN + 2: return 11;
    case LLONG_MIN + 5: return -7;
    default: return 23;
    }
}

NOINLINE int i64_high_two_val(long long x) {
    switch (x) {
    case LLONG_MAX - 5: return -7;
    case LLONG_MAX - 2: return 11;
    case LLONG_MAX: return -7;
    default: return 23;
    }
}

NOINLINE int u64_top_membership(unsigned long long x) {
    switch (x) {
    case ULLONG_MAX - 2: return 1;
    case ULLONG_MAX - 1: return 0;
    case ULLONG_MAX: return 1;
    default: return 0;
    }
}

static unsigned long long h = 1469598103934665603ULL;
static void mix(int x) {
    h ^= (unsigned long long)x + 0x9e3779b97f4a7c15ULL + (h << 6) + (h >> 2);
    h *= 1099511628211ULL;
}

int main(void) {
    const long long lows[] = {LLONG_MIN,      LLONG_MIN + 1, LLONG_MIN + 2,
                              LLONG_MIN + 3,  LLONG_MIN + 5, LLONG_MIN + 6,
                              LLONG_MIN + 64, 0,             LLONG_MAX};
    const long long highs[] = {LLONG_MAX - 6,  LLONG_MAX - 5, LLONG_MAX - 2,
                               LLONG_MAX - 1,  LLONG_MAX,     LLONG_MAX - 64,
                               LLONG_MIN,      0};
    const unsigned long long tops[] = {0, 1, 63, 64, ULLONG_MAX - 3,
                                       ULLONG_MAX - 2, ULLONG_MAX - 1, ULLONG_MAX};
    for (unsigned i = 0; i < sizeof(lows) / sizeof(lows[0]); ++i)
        mix(i64_low_two_val(lows[i]));
    for (unsigned i = 0; i < sizeof(highs) / sizeof(highs[0]); ++i)
        mix(i64_high_two_val(highs[i]));
    for (unsigned i = 0; i < sizeof(tops) / sizeof(tops[0]); ++i)
        mix(u64_top_membership(tops[i]));
    printf("%016llx\n", h);
    return h != 0xbdddfb537d009427ULL;
}
