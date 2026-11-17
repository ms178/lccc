/* _mm256_max_epi16 / _mm256_min_epi16 on directed and pseudo-random 16-bit
 * lanes (-O2 -mavx2). Each lane must equal the scalar SIGNED max/min. The
 * inputs include the sign boundary (0x7FFF = +32767 vs 0x8000 = -32768): an
 * unsigned compare disagrees with the signed one exactly there, so a lowering
 * that swapped vpmaxsw for vpmaxuw fails this test. The program prints the
 * mismatch count and a checksum; GCC 16.2 must print the same line, and the
 * exit status is 0 only when the mismatch count is 0. */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

static short lanes_a[512];
static short lanes_b[512];

__attribute__((noinline)) static __m256i load16(const short *p)
{
    __m256i v;
    memcpy(&v, p, sizeof v);
    return v;
}

__attribute__((noinline)) static void store16(short *p, __m256i v)
{
    memcpy(p, &v, sizeof v);
}

int main(void)
{
    static const short edge[] = {0x0000, 0x0001, 0x7FFF, (short)0x8000,
                                 (short)0x8001, (short)0xFFFE, (short)0xFFFF,
                                 0x1234};
    unsigned int state = 0x6A09E667u;
    for (int i = 0; i < 512; i++) {
        state = state * 1664525u + 1013904223u;
        lanes_a[i] = (short)(state >> 16);
        state = state * 1664525u + 1013904223u;
        lanes_b[i] = (short)(state >> 16);
    }
    /* Every ordered pair of boundary values occupies the first 64 lanes. */
    for (int i = 0; i < 64; i++) {
        lanes_a[i] = edge[i / 8];
        lanes_b[i] = edge[i % 8];
    }

    unsigned long long mismatches = 0;
    unsigned long long checksum = 0;
    for (int group = 0; group < 32; group++) {
        const short *pa = &lanes_a[group * 16];
        const short *pb = &lanes_b[group * 16];
        __m256i a = load16(pa);
        __m256i b = load16(pb);
        short mx[16], mn[16];
        store16(mx, _mm256_max_epi16(a, b));
        store16(mn, _mm256_min_epi16(a, b));
        for (int lane = 0; lane < 16; lane++) {
            short x = pa[lane];
            short y = pb[lane];
            short want_max = x > y ? x : y;
            short want_min = x < y ? x : y;
            if (mx[lane] != want_max)
                mismatches++;
            if (mn[lane] != want_min)
                mismatches++;
            checksum = checksum * 1000003ull + (unsigned short)mx[lane] +
                       3ull * (unsigned short)mn[lane];
        }
    }
    printf("avx2 epi16 min/max: mismatches=%llu checksum=%016llx\n", mismatches,
           checksum);
    return mismatches == 0 ? 0 : 1;
}
