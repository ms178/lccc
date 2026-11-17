/* _mm_max_epu16 / _mm_min_epu16 on directed and pseudo-random 16-bit lanes
 * (-O2 -msse4.1). Each lane must equal the scalar unsigned max/min. The inputs
 * include the sign-bit boundary (0x7FFF/0x8000), which is where a signed
 * compare (pmaxsw/pminsw) would disagree, and the all-ones and zero lanes.
 * The program prints the mismatch count and a checksum of the results; GCC
 * 16.2 must print the same line, and the exit status is 0 only when the
 * mismatch count is 0. */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

static unsigned short lanes_a[8 * 64];
static unsigned short lanes_b[8 * 64];

__attribute__((noinline)) static __m128i load16(const unsigned short *p)
{
    __m128i v;
    memcpy(&v, p, sizeof v);
    return v;
}

__attribute__((noinline)) static void store16(unsigned short *p, __m128i v)
{
    memcpy(p, &v, sizeof v);
}

int main(void)
{
    static const unsigned short edge[] = {0x0000, 0x0001, 0x7FFF, 0x8000,
                                          0x8001, 0xFFFE, 0xFFFF, 0x1234};
    unsigned int state = 0x2545F491u;
    for (int i = 0; i < 8 * 64; i++) {
        state = state * 1664525u + 1013904223u;
        lanes_a[i] = (unsigned short)(state >> 16);
        state = state * 1664525u + 1013904223u;
        lanes_b[i] = (unsigned short)(state >> 16);
    }
    /* Edge lanes first, so every ordered pair of boundary values is covered. */
    for (int i = 0; i < 64; i++) {
        lanes_a[i] = edge[i / 8];
        lanes_b[i] = edge[i % 8];
    }

    unsigned long long mismatches = 0;
    unsigned long long checksum = 0;
    for (int block = 0; block < 8; block++) {
        const unsigned short *pa = &lanes_a[block * 8 * 8];
        const unsigned short *pb = &lanes_b[block * 8 * 8];
        for (int group = 0; group < 8; group++) {
            __m128i a = load16(pa + group * 8);
            __m128i b = load16(pb + group * 8);
            unsigned short mx[8], mn[8];
            store16(mx, _mm_max_epu16(a, b));
            store16(mn, _mm_min_epu16(a, b));
            for (int lane = 0; lane < 8; lane++) {
                unsigned short x = pa[group * 8 + lane];
                unsigned short y = pb[group * 8 + lane];
                unsigned short want_max = x > y ? x : y;
                unsigned short want_min = x < y ? x : y;
                if (mx[lane] != want_max)
                    mismatches++;
                if (mn[lane] != want_min)
                    mismatches++;
                checksum = checksum * 1000003ull + mx[lane] + 3ull * mn[lane];
            }
        }
    }
    printf("sse41 epu16 min/max: mismatches=%llu checksum=%016llx\n", mismatches,
           checksum);
    return mismatches == 0 ? 0 : 1;
}
