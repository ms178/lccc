/* zlib-ng / gzip CRC32 slicing-by-8 (generic path, crc32_braid algorithm
 * shape): 8 table lookups + XORs per 8 bytes, tables built at setup from the
 * reflected IEEE polynomial so no 8 KiB literal table rides in the source.
 * Exercises load-port pressure, XOR chains, unaligned 64-bit loads (done as
 * memcpy so the compiler may fold them) and table-cache locality -- the
 * non-SIMD baseline every PCLMULQDQ version is measured against. */
#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include "bench.h"

#define N 8192
static unsigned char buf[N];
static uint32_t tab[8][256];

void bench_setup(void) {
    for (int i = 0; i < N; i++) buf[i] = (unsigned char)(i * 167 + (i >> 3) + 11);
    for (uint32_t n = 0; n < 256; n++) {
        uint32_t c = n;
        for (int k = 0; k < 8; k++) c = (c & 1u) ? (0xEDB88320u ^ (c >> 1)) : (c >> 1);
        tab[0][n] = c;
    }
    for (int t = 1; t < 8; t++)
        for (int n = 0; n < 256; n++) {
            uint32_t c = tab[t - 1][n];
            tab[t][n] = tab[0][c & 0xffu] ^ (c >> 8);
        }
}

static uint32_t crc32_slice8(uint32_t crc, const unsigned char *b, size_t len) {
    crc = ~crc;
    size_t i = 0;
    for (; i + 8 <= len; i += 8) {
        uint64_t w;
        memcpy(&w, b + i, 8);
        uint32_t lo = (uint32_t)w ^ crc;
        uint32_t hi = (uint32_t)(w >> 32);
        crc = tab[7][lo & 0xffu] ^ tab[6][(lo >> 8) & 0xffu]
            ^ tab[5][(lo >> 16) & 0xffu] ^ tab[4][lo >> 24]
            ^ tab[3][hi & 0xffu] ^ tab[2][(hi >> 8) & 0xffu]
            ^ tab[1][(hi >> 16) & 0xffu] ^ tab[0][hi >> 24];
    }
    for (; i < len; i++) crc = tab[0][(crc ^ b[i]) & 0xffu] ^ (crc >> 8);
    return ~crc;
}

int bench_verify(void) {
    /* CRC-32/ISO-HDLC canonical vectors: empty input, the universal check
     * string, a 32-byte zero run, and the pangram.  "123456789" (9 bytes)
     * exercises one slicing step plus the scalar tail.  A broken table
     * build or a mis-transcribed polynomial is deterministic, so every
     * compiler arm would agree on the wrong bench checksum -- only an
     * absolute oracle like this catches it. */
    static const unsigned char zeros[32] = {0};
    if (crc32_slice8(0, zeros, 0) != 0x00000000u) return 1;
    if (crc32_slice8(0, (const unsigned char *)"123456789", 9) != 0xCBF43926u)
        return 2;
    if (crc32_slice8(0, zeros, sizeof(zeros)) != 0x190A55ADu) return 3;
    if (crc32_slice8(0, (const unsigned char *)"The quick brown fox jumps over the lazy dog", 43)
            != 0x414FA339u)
        return 4;
    return 0;
}

unsigned long long bench_run(void) { return crc32_slice8(0, buf, N); }
