/* Dispatch-shaped workload: one runtime feature test decides the inner loop,
 * like zlib-ng/zstd/xz/OpenSSL.  The kernel body is loop-carried and
 * deterministic so LCCC and GCC binaries can be compared instruction-for-
 * instruction with callgrind.
 */
#include <stdio.h>
#include <stdint.h>

static uint32_t table[256];
static void init_table(void) {
    uint32_t c;
    int n, k;
    for (n = 0; n < 256; n++) {
        c = (uint32_t)n;
        for (k = 0; k < 8; k++) c = (c & 1) ? 0xEDB88320u ^ (c >> 1) : (c >> 1);
        table[n] = c;
    }
}

static uint32_t crc32_byte(const unsigned char *p, long n) {
    uint32_t crc = 0xFFFFFFFFu;
    long i;
    for (i = 0; i < n; i++) crc = table[(crc ^ p[i]) & 0xFF] ^ (crc >> 8);
    return crc ^ 0xFFFFFFFFu;
}

static uint32_t crc32_word(const unsigned char *p, long n) {
    uint32_t crc = 0xFFFFFFFFu;
    long i;
    for (i = 0; i + 4 <= n; i += 4) {
        uint32_t w;
        __builtin_memcpy(&w, p + i, 4);
        crc = table[(crc ^ w) & 0xFF] ^ (crc >> 8);
        crc = table[(crc ^ (w >> 8)) & 0xFF] ^ (crc >> 8);
        crc = table[(crc ^ (w >> 16)) & 0xFF] ^ (crc >> 8);
        crc = table[(crc ^ (w >> 24)) & 0xFF] ^ (crc >> 8);
    }
    for (; i < n; i++) crc = table[(crc ^ p[i]) & 0xFF] ^ (crc >> 8);
    return crc ^ 0xFFFFFFFFu;
}

int main(void) {
    static unsigned char buf[1 << 16];
    long i, rounds = 4000;
    uint32_t acc = 0;
    init_table();
    for (i = 0; i < (long)sizeof(buf); i++) buf[i] = (unsigned char)(i * 31 + 7);
    /* The dispatch site: exactly one runtime query, both paths integer-only
     * (the vector path is what a real dispatcher would select). */
    if (__builtin_cpu_supports("avx2") || __builtin_cpu_supports("sse2")) {
        for (i = 0; i < rounds; i++) acc ^= crc32_word(buf, sizeof(buf));
    } else {
        for (i = 0; i < rounds; i++) acc ^= crc32_byte(buf, sizeof(buf));
    }
    printf("%u\n", acc);
    return 0;
}
