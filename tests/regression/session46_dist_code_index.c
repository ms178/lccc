/* Session-46 reproducer: zlib-ng trees_emit.h zng_emit_dist partial-register
 * codegen bug. `code` is uint8_t, assigned from a 32-bit computation, then
 * used as an index into a const int[] table (scale-4 SIB). The copy into
 * code's register home must be zero-extended; a byte move leaves the upper
 * 56 bits stale and the folded index reads garbage (OOB). */
#include <stdint.h>
#include <stdio.h>

static const int extra_lbits[29] = {0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2,
                                    2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0};
static const int extra_dbits[30] = {0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6,
                                    6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13};
static const int base_length[29] = {0, 1, 2, 3, 4, 5, 6, 7, 8, 10, 12, 14, 16, 20,
                                    24, 28, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160,
                                    192, 224, 0};
static const int base_dist[30] = {0, 1, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64, 96,
                                  128, 192, 256, 384, 512, 768, 1024, 1536, 2048, 3072,
                                  4096, 6144, 8192, 12288, 16384, 24576};
static uint8_t dist_code[512];
static uint8_t length_code[256];

static uint32_t d_code(uint32_t dist) {
    return dist < 256 ? dist_code[dist] : dist_code[256 + (dist >> 7)];
}

__attribute__((noinline)) uint32_t
emit_dist(uint32_t lc, uint32_t dist, uint64_t match_bits,
          unsigned match_bits_len) {
    uint8_t code;
    uint32_t extra;

    code = length_code[lc & 255];
    extra = extra_lbits[code];
    if (extra != 0) {
        lc -= (uint32_t)base_length[code];
        match_bits |= ((uint64_t)lc << match_bits_len);
        match_bits_len += extra;
    }

    dist--;
    code = d_code(dist);
    extra = (uint32_t)extra_dbits[code];
    if (extra != 0) {
        dist -= (uint32_t)base_dist[code];
        match_bits |= ((uint64_t)dist << match_bits_len);
        match_bits_len += extra;
    }
    return match_bits_len;
}

static uint32_t ref_emit(uint32_t lc, uint32_t dist, uint64_t match_bits,
                         unsigned match_bits_len) {
    uint32_t code, extra;
    code = length_code[lc & 255];
    extra = extra_lbits[code];
    if (extra != 0) {
        lc -= (uint32_t)base_length[code];
        match_bits |= ((uint64_t)lc << match_bits_len);
        match_bits_len += extra;
    }
    dist--;
    code = d_code(dist);
    extra = (uint32_t)extra_dbits[code];
    if (extra != 0) {
        dist -= (uint32_t)base_dist[code];
        match_bits |= ((uint64_t)dist << match_bits_len);
        match_bits_len += extra;
    }
    return match_bits_len;
}

int main(void) {
    for (int i = 0; i < 256; i++)
        length_code[i] = (uint8_t)(i % 29);
    for (int i = 0; i < 512; i++)
        dist_code[i] = (uint8_t)(i % 30);
    uint32_t seed = 12345;
    for (uint32_t d = 1; d < 32768; d = d * 3 + 1) {
        for (uint32_t l = 3; l < 258; l = l * 5 + 1) {
            uint64_t bits = seed;
            uint32_t got = emit_dist(l, d, bits, 0);
            uint32_t ref = ref_emit(l, d, bits, 0);
            if (got != ref) {
                printf("FAIL l=%u d=%u got=%u ref=%u\n", l, d, got, ref);
                return 1;
            }
            seed = seed * 1103515245u + 12345u;
        }
    }
    printf("OK dist-code index\n");
    return 0;
}
