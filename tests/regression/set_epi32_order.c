/* _mm256_set_epi32 argument order vs memory order.
 *
 * The header macro lowerings must place the LAST argument in the LOWEST
 * address (little-endian dword order), exactly like GCC. These are the
 * folded constants used by zlib-ng's CRC fold: a wrong order flips the
 * 32-bit halves of every 64-bit fold constant and corrupts the CRC. */
#include <immintrin.h>
#include <stdio.h>
#include <stdint.h>
#include <string.h>

int main(void) {
    __m256i fold4 = _mm256_set_epi32(
        0x00000001, 0x54442bd4, 0x00000001, 0xc6e41596,
        0x00000001, 0x54442bd4, 0x00000001, 0xc6e41596);
    __m256i fold8 = _mm256_set_epi32(
        0x00000001, 0xe88ef372, 0x00000001, 0x4a7fe880,
        0x00000001, 0xe88ef372, 0x00000001, 0x4a7fe880);

    /* Expected LE dword order: last argument first in memory. */
    uint32_t want4[8] = {
        0xc6e41596, 0x00000001, 0x54442bd4, 0x00000001,
        0xc6e41596, 0x00000001, 0x54442bd4, 0x00000001};
    uint32_t got4[8];
    memcpy(got4, &fold4, 32);
    int bad = 0;
    for (int i = 0; i < 8; i++) if (got4[i] != want4[i]) bad = 1;

    uint32_t want8[8] = {
        0x4a7fe880, 0x00000001, 0xe88ef372, 0x00000001,
        0x4a7fe880, 0x00000001, 0xe88ef372, 0x00000001};
    uint32_t got8[8];
    memcpy(got8, &fold8, 32);
    for (int i = 0; i < 8; i++) if (got8[i] != want8[i]) bad = 1;

    /* General form with distinct values, compared against GCC semantics:
     * _mm256_set_epi32(e7,...,e0) stores e0 at the lowest address. */
    __m256i g = _mm256_set_epi32(0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17);
    uint32_t wantg[8] = {0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11, 0x10};
    uint32_t gotg[8];
    memcpy(gotg, &g, 32);
    for (int i = 0; i < 8; i++) if (gotg[i] != wantg[i]) bad = 1;

    if (bad) { printf("FAILED\n"); return 1; }
    printf("OK\n");
    return 0;
}
