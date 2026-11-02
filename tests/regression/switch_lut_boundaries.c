#include <limits.h>
#include <stdint.h>
#include <stdio.h>

#define NOINLINE __attribute__((noinline))

NOINLINE int sc(signed char x) {
    switch (x) {
    case -128: return -5;
    case -127: return -4;
    case -126: return -3;
    default: return 99;
    }
}

NOINLINE unsigned int u32top(unsigned int x) {
    switch (x) {
    case UINT_MAX - 2: return UINT_MAX - 10;
    case UINT_MAX - 1: return UINT_MAX - 9;
    case UINT_MAX: return UINT_MAX - 8;
    default: return 7;
    }
}

NOINLINE long long i64small(long long x) {
    switch (x) {
    case -3: return -50;
    case -1: return 10;
    case 2: return 30;
    default: return -40;
    }
}

NOINLINE unsigned long long u64top(unsigned long long x) {
    switch (x) {
    case ULLONG_MAX - 2: return ULLONG_MAX - 20;
    case ULLONG_MAX - 1: return ULLONG_MAX - 19;
    case ULLONG_MAX: return ULLONG_MAX - 18;
    default: return 7;
    }
}

NOINLINE unsigned long long uret(int x) {
    switch (x) {
    case 0: return ULLONG_MAX - 2;
    case 1: return ULLONG_MAX - 1;
    case 2: return ULLONG_MAX;
    default: return 7;
    }
}

NOINLINE int join(int flag, int x) {
    int r;
    if (flag) {
        r = 777;
    } else {
        switch (x) {
        case -2: r = -50; break;
        case 0: r = 10; break;
        case 2: r = 30; break;
        default: r = -40; break;
        }
    }
    return r * 3 + 1;
}

static unsigned long long h = 1469598103934665603ULL;
static void mix(unsigned long long x) {
    h ^= x + 0x9e3779b97f4a7c15ULL + (h << 6) + (h >> 2);
    h *= 1099511628211ULL;
}

int main(void) {
    for (int x = -128; x <= 127; ++x) mix((unsigned)sc((signed char)x));
    const unsigned int u32s[] = {0, 1, 2, UINT_MAX - 3, UINT_MAX - 2,
                                 UINT_MAX - 1, UINT_MAX};
    for (unsigned i = 0; i < sizeof(u32s) / sizeof(u32s[0]); ++i)
        mix(u32top(u32s[i]));
    const long long i64s[] = {-5, -3, -2, -1, 0, 1, 2, 3, LLONG_MIN, LLONG_MAX};
    for (unsigned i = 0; i < sizeof(i64s) / sizeof(i64s[0]); ++i)
        mix((unsigned long long)i64small(i64s[i]));
    const unsigned long long u64s[] = {0, 1, 2, ULLONG_MAX - 3, ULLONG_MAX - 2,
                                       ULLONG_MAX - 1, ULLONG_MAX};
    for (unsigned i = 0; i < sizeof(u64s) / sizeof(u64s[0]); ++i)
        mix(u64top(u64s[i]));
    for (int x = -3; x <= 4; ++x) mix(uret(x));
    for (int f = 0; f < 2; ++f)
        for (int x = -5; x <= 5; ++x) mix((unsigned)join(f, x));
    printf("%016llx\n", h);
    return h != 0x9c7056685b47ab71ULL;
}
