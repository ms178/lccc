/* Regression test for CRC32 table-index narrowing across signed/unsigned mask,
 * narrow cast, nibble, and shifted index spellings, plus exhaustive 256x256
 * byte-index verification.
 */
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>

static uint32_t crc_table[256];
static uint32_t nibble_table[16];

static void init_tables(void) {
    for (uint32_t i = 0; i < 256; ++i) {
        uint32_t c = i;
        for (int k = 0; k < 8; ++k)
            c = (c & 1u) ? (0xEDB88320u ^ (c >> 1)) : (c >> 1);
        crc_table[i] = c;
    }
    for (uint32_t i = 0; i < 16; ++i)
        nibble_table[i] = crc_table[i * 16];
}

__attribute__((noinline))
uint32_t crc_signed_mask_ptr(uint32_t crc, const uint8_t *p, size_t n) {
    const uint8_t *end = p + n;
    while (p != end) {
        int idx = (int)((crc ^ *p++) & 0xffu);
        crc = crc_table[idx] ^ (crc >> 8);
    }
    return crc;
}

__attribute__((noinline))
uint32_t crc_u8_cast_idx(uint32_t crc, const uint8_t *p, size_t n) {
    for (size_t i = 0; i < n; ++i) {
        uint8_t idx = (uint8_t)(crc ^ p[i]);
        crc = crc_table[idx] ^ (crc >> 8);
    }
    return crc;
}

__attribute__((noinline))
uint32_t crc_signed_char_input(uint32_t crc, const signed char *p, int n) {
    for (int i = 0; i < n; ++i) {
        int idx = ((int)crc ^ (int)(unsigned char)p[i]) & 255;
        crc = crc_table[idx] ^ (crc >> 8);
    }
    return crc;
}

__attribute__((noinline))
uint32_t crc_nibble_signed_mask(uint32_t crc, const uint8_t *p, size_t n) {
    for (size_t i = 0; i < n; ++i) {
        int idx = ((int)crc ^ (int)p[i]) & 15;
        crc = nibble_table[idx] ^ (crc >> 4);
    }
    return crc;
}

int main(void) {
    init_tables();
    static const uint8_t msg[] = "123456789";
    uint32_t a = crc_signed_mask_ptr(0xFFFFFFFFu, msg, 9) ^ 0xFFFFFFFFu;
    uint32_t b = crc_u8_cast_idx(0xFFFFFFFFu, msg, 9) ^ 0xFFFFFFFFu;
    uint32_t c = crc_signed_char_input(0xFFFFFFFFu, (const signed char *)msg, 9) ^ 0xFFFFFFFFu;
    uint32_t d = crc_nibble_signed_mask(0u, msg, 9);
    if (a != 0xCBF43926u || b != 0xCBF43926u || c != 0xCBF43926u) {
        printf("FAIL %08x %08x %08x\n", a, b, c);
        return 1;
    }

    /* Exhaustive 256-byte index sweep across high-bit-set CRC seeds. */
    uint8_t all_bytes[256];
    for (unsigned i = 0; i < 256; ++i)
        all_bytes[i] = (uint8_t)i;
    for (unsigned hi = 0; hi < 256; ++hi) {
        uint32_t seed = 0x80000000u ^ (hi << 24) ^ (hi * 0x01010101u);
        uint32_t r1 = crc_signed_mask_ptr(seed, all_bytes, 256);
        uint32_t r2 = crc_u8_cast_idx(seed, all_bytes, 256);
        uint32_t r3 = crc_signed_char_input(seed, (const signed char *)all_bytes, 256);
        if (r1 != r2 || r1 != r3) {
            printf("FAIL sweep hi=%u: %08x %08x %08x\n", hi, r1, r2, r3);
            return 2;
        }
    }
    printf("%08x %08x\n", a, d);
    return 0;
}
