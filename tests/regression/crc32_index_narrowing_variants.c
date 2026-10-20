/* Regression test for CRC32 table-index narrowing across signed/unsigned mask,
 * narrow cast, nibble, and shifted index spellings, plus independent one-step Cartesian byte-index verification.
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

static uint32_t reference_byte(uint32_t crc, uint8_t byte) {
    crc ^= byte;
    for (unsigned bit = 0; bit < 8; ++bit)
        crc = (crc >> 1) ^ ((crc & 1u) ? 0xEDB88320u : 0u);
    return crc;
}

static uint32_t reference_nibble(uint32_t crc, uint8_t byte) {
    /* This witness uses table[idx*16], not the conventional nibble table.
     * Reconstruct that entry independently without using either table. */
    uint32_t entry = ((crc ^ byte) & 15u) << 4;
    for (unsigned bit = 0; bit < 8; ++bit)
        entry = (entry >> 1) ^ ((entry & 1u) ? 0xEDB88320u : 0u);
    return entry ^ (crc >> 4);
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

    /* Each pair is an independent one-byte update, not evolving CRC state. */
    static const uint32_t upper[] = {0u, 0x80000000u, 0xffffff00u, 0x12345600u};
    for (unsigned h = 0; h < sizeof(upper) / sizeof(upper[0]); ++h) {
        for (unsigned low = 0; low < 256; ++low) {
            for (unsigned byte = 0; byte < 256; ++byte) {
                uint32_t seed = upper[h] | low;
                uint8_t input = (uint8_t)byte;
                uint32_t ref = reference_byte(seed, input);
                if (crc_signed_mask_ptr(seed, &input, 1) != ref ||
                    crc_u8_cast_idx(seed, &input, 1) != ref ||
                    crc_signed_char_input(seed, (const signed char *)&input, 1) != ref ||
                    crc_nibble_signed_mask(seed, &input, 1) != reference_nibble(seed, input)) return 3;
            }
        }
    }

    /* Multi-byte recurrence checks complement the Cartesian one-step sweep. */
    uint8_t all_bytes[256];
    for (unsigned i = 0; i < 256; ++i)
        all_bytes[i] = (uint8_t)i;
    for (unsigned hi = 0; hi < 256; ++hi) {
        uint32_t seed = 0x80000000u ^ (hi << 24) ^ (hi * 0x01010101u);
        uint32_t r1 = crc_signed_mask_ptr(seed, all_bytes, 256);
        uint32_t r2 = crc_u8_cast_idx(seed, all_bytes, 256);
        uint32_t r3 = crc_signed_char_input(seed, (const signed char *)all_bytes, 256);
        uint32_t ref = seed;
        for (unsigned i = 0; i < 256; ++i) ref = reference_byte(ref, all_bytes[i]);
        if (r1 != ref || r2 != ref || r3 != ref) {
            printf("FAIL sweep hi=%u: %08x %08x %08x\n", hi, r1, r2, r3);
            return 2;
        }
    }
    printf("%08x %08x\n", a, d);
    return 0;
}
