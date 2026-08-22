/* Session-46 narrow-cast torture: every narrowing/same-size cast must leave
 * the destination register defined at FULL width, because x86 code may read
 * it at a wider width later (folded SIB indices, movq copies, 64-bit
 * consumers). Exercises U32/U64 -> U8/U16/I8/I16 through: table indexing
 * (scale 1/2/4), shifts, mul, phi joins, selects, and call boundaries. */
#include <stdint.h>
#include <stdio.h>

static const int32_t tbl4[32] = {
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
    -16, -15, -14, -13, -12, -11, -10, -9, -8, -7, -6, -5, -4, -3, -2, -1};
static int16_t tbl2[64];
static int8_t tbl1[64];

__attribute__((noinline)) uint32_t idx_u8(uint32_t x) {
    uint8_t c = (uint8_t)x;
    return (uint32_t)tbl4[c];
}
__attribute__((noinline)) uint32_t idx_i8(int32_t x) {
    int8_t c = (int8_t)x;
    return (uint32_t)tbl4[c & 31];
}
__attribute__((noinline)) uint32_t idx_u16(uint32_t x) {
    uint16_t c = (uint16_t)x;
    return (uint32_t)tbl2[c & 63];
}
__attribute__((noinline)) uint32_t idx_i16(int32_t x) {
    int16_t c = (int16_t)x;
    return (uint32_t)tbl4[c & 31];
}
__attribute__((noinline)) uint32_t shl_u8(uint32_t x) {
    uint8_t c = (uint8_t)x;
    return (uint32_t)c << 24;
}
__attribute__((noinline)) uint32_t mul_u8(uint32_t x) {
    uint8_t c = (uint8_t)x;
    return (uint32_t)c * 977u;
}
__attribute__((noinline)) uint32_t sel_u8(uint32_t x, uint32_t y) {
    uint8_t c = (uint8_t)(x > y ? x : y);
    return (uint32_t)tbl4[c & 31];
}
__attribute__((noinline)) int32_t narrow_i8_call(int32_t x) {
    int8_t c = (int8_t)x;
    return (int32_t)c * 3; /* sign semantics across widen */
}
__attribute__((noinline)) uint32_t chain(uint32_t x) {
    uint8_t a = (uint8_t)x;        /* U32->U8 */
    uint16_t b = (uint16_t)a;      /* U8->U16 */
    uint32_t d = (uint32_t)b;      /* U16->U32 */
    return d * 13u + (uint32_t)tbl4[a & 31];
}

int main(void) {
    for (int i = 0; i < 64; i++) {
        tbl2[i] = (int16_t)(i * 7 - 100);
        tbl1[i] = (int8_t)(i - 32);
    }
    uint32_t seed = 1;
    int bad = 0;
    for (uint32_t i = 0; i < 200000; i++) {
        uint32_t x = seed;
        seed = seed * 1103515245u + 12345u;
        uint32_t y = seed;
        seed = seed * 1103515245u + 12345u;
        int32_t sx = (int32_t)(x ^ 0x80000000u);
        uint8_t c8 = (uint8_t)x;
        uint16_t c16 = (uint16_t)x;
        int8_t s8 = (int8_t)sx;
        int16_t s16 = (int16_t)sx;
        if (idx_u8(x) != (uint32_t)tbl4[c8]) { printf("FAIL idx_u8 %u\n", x); bad = 1; break; }
        if (idx_i8(sx) != (uint32_t)tbl4[s8 & 31]) { printf("FAIL idx_i8 %d\n", sx); bad = 1; break; }
        if (idx_u16(x) != (uint32_t)tbl2[c16 & 63]) { printf("FAIL idx_u16 %u\n", x); bad = 1; break; }
        if (idx_i16(sx) != (uint32_t)tbl4[s16 & 31]) { printf("FAIL idx_i16 %d\n", sx); bad = 1; break; }
        if (shl_u8(x) != (uint32_t)c8 << 24) { printf("FAIL shl_u8 %u\n", x); bad = 1; break; }
        if (mul_u8(x) != (uint32_t)c8 * 977u) { printf("FAIL mul_u8 %u\n", x); bad = 1; break; }
        if (sel_u8(x, y) != (uint32_t)tbl4[((uint8_t)(x > y ? x : y)) & 31]) { printf("FAIL sel_u8 %u %u\n", x, y); bad = 1; break; }
        if (narrow_i8_call(sx) != (int32_t)s8 * 3) { printf("FAIL narrow_i8_call %d\n", sx); bad = 1; break; }
        if (chain(x) != c8 * 13u + (uint32_t)tbl4[c8 & 31]) { printf("FAIL chain %u\n", x); bad = 1; break; }
    }
    if (!bad)
        printf("OK narrow-cast torture\n");
    return bad;
}
