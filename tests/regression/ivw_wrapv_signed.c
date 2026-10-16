/* Compile with -fwrapv. Independent Python integer/ring oracle: expected
   hash below, over 864 signed-limit trajectories and two product loops.
   Every index is masked, including deliberately wrapping boundary paths. */
#include <stdint.h>
#include <stdio.h>
#define NOINLINE __attribute__((noinline))
#define BODY do { sum = sum * 33U + p[(i & 63) + 1]; } while (0)
NOINLINE static unsigned asc(const unsigned *p, int seed, int end) {
    unsigned sum = 0, trips = 0;
    for (int i = seed; i < end; i += (1)) {
        BODY; if (++trips == 32) break;
    }
    return sum;
}
NOINLINE static unsigned asc3(const unsigned *p, int seed, int end) {
    unsigned sum = 0, trips = 0;
    for (int i = seed; i < end; i += (3)) {
        BODY; if (++trips == 32) break;
    }
    return sum;
}
NOINLINE static unsigned asc_inclusive(const unsigned *p, int seed, int end) {
    unsigned sum = 0, trips = 0;
    for (int i = seed; i <= end; i += (1)) {
        BODY; if (++trips == 32) break;
    }
    return sum;
}
NOINLINE static unsigned asc3_inclusive(const unsigned *p, int seed, int end) {
    unsigned sum = 0, trips = 0;
    for (int i = seed; i <= end; i += (3)) {
        BODY; if (++trips == 32) break;
    }
    return sum;
}
NOINLINE static unsigned desc(const unsigned *p, int seed, int end) {
    unsigned sum = 0, trips = 0;
    for (int i = seed; i > end; i += (-1)) {
        BODY; if (++trips == 32) break;
    }
    return sum;
}
NOINLINE static unsigned desc3(const unsigned *p, int seed, int end) {
    unsigned sum = 0, trips = 0;
    for (int i = seed; i > end; i += (-3)) {
        BODY; if (++trips == 32) break;
    }
    return sum;
}
NOINLINE static unsigned desc_inclusive(const unsigned *p, int seed, int end) {
    unsigned sum = 0, trips = 0;
    for (int i = seed; i >= end; i += (-1)) {
        BODY; if (++trips == 32) break;
    }
    return sum;
}
NOINLINE static unsigned desc3_inclusive(const unsigned *p, int seed, int end) {
    unsigned sum = 0, trips = 0;
    for (int i = seed; i >= end; i += (-3)) {
        BODY; if (++trips == 32) break;
    }
    return sum;
}
NOINLINE static unsigned post_asc(const unsigned *p, int seed, int end) {
    unsigned sum = 0, trips = 0;
    int i = seed;
    do { BODY; if (++trips == 32) break; i += (1); } while (i < end);
    return sum;
}
NOINLINE static unsigned post_desc(const unsigned *p, int seed, int end) {
    unsigned sum = 0, trips = 0;
    int i = seed;
    do { BODY; if (++trips == 32) break; i += (-1); } while (i > end);
    return sum;
}
NOINLINE static unsigned post_asc3(const unsigned *p, int seed, int end) {
    unsigned sum = 0, trips = 0;
    int i = seed;
    do { BODY; if (++trips == 32) break; i += (3); } while (i <= end);
    return sum;
}
NOINLINE static unsigned post_desc3(const unsigned *p, int seed, int end) {
    unsigned sum = 0, trips = 0;
    int i = seed;
    do { BODY; if (++trips == 32) break; i += (-3); } while (i >= end);
    return sum;
}
NOINLINE static unsigned products_asc(const unsigned *p) {
    unsigned sum = 0;
    for (int i = (-63); i <= (63); i += (1))
        sum = sum * 33U + p[(unsigned)(i * (i + 1)) & 63U];
    return sum;
}
NOINLINE static unsigned products_desc(const unsigned *p) {
    unsigned sum = 0;
    for (int i = (63); i >= (-63); i += (-1))
        sum = sum * 33U + p[(unsigned)(i * (i + 1)) & 63U];
    return sum;
}
int main(void) {
    unsigned p[65];
    const int seeds[] = { (-2147483647 - 1), -2147483647, -65, -1, 0, 1, 63, 2147483646, 2147483647 };
    const int limits[] = { (-2147483647 - 1), -2147483647, -64, -1, 0, 64, 2147483646, 2147483647 };
    unsigned h = 2166136261U;
    for (unsigned i = 0; i < 65; ++i) p[i] = i * 37U + 11U;
    for (unsigned s = 0; s < sizeof(seeds) / sizeof(seeds[0]); ++s)
        for (unsigned e = 0; e < sizeof(limits) / sizeof(limits[0]); ++e) {
            h = (h ^ asc(p, seeds[s], limits[e])) * 16777619U;
            h = (h ^ asc3(p, seeds[s], limits[e])) * 16777619U;
            h = (h ^ asc_inclusive(p, seeds[s], limits[e])) * 16777619U;
            h = (h ^ asc3_inclusive(p, seeds[s], limits[e])) * 16777619U;
            h = (h ^ desc(p, seeds[s], limits[e])) * 16777619U;
            h = (h ^ desc3(p, seeds[s], limits[e])) * 16777619U;
            h = (h ^ desc_inclusive(p, seeds[s], limits[e])) * 16777619U;
            h = (h ^ desc3_inclusive(p, seeds[s], limits[e])) * 16777619U;
            h = (h ^ post_asc(p, seeds[s], limits[e])) * 16777619U;
            h = (h ^ post_desc(p, seeds[s], limits[e])) * 16777619U;
            h = (h ^ post_asc3(p, seeds[s], limits[e])) * 16777619U;
            h = (h ^ post_desc3(p, seeds[s], limits[e])) * 16777619U;
        }
    h = (h ^ products_asc(p)) * 16777619U;
    h = (h ^ products_desc(p)) * 16777619U;
    if (h != 3590369908U) { printf("ivw_wrapv_signed: FAIL %u\n", h); return 1; }
    puts("ivw_wrapv_signed: PASS");
    return 0;
}
