#include <stdint.h>
#include <stdio.h>

__attribute__((noinline)) static unsigned classify_u16(uint16_t value) {
    unsigned result = 0;
    result |= value == (uint16_t)-2 ? 1u : 0u;
    result |= value != (uint16_t)-3 ? 2u : 0u;
    result |= value < (uint16_t)-4 ? 4u : 0u;
    result |= value >= (uint16_t)-5 ? 8u : 0u;
    return result;
}

__attribute__((noinline)) static unsigned classify_u8(uint8_t value) {
    return (value == (uint8_t)-2 ? 1u : 0u)
         | (value < (uint8_t)-1 ? 2u : 0u);
}

__attribute__((noinline)) static unsigned classify_i16(int16_t value) {
    return (value == -2 ? 1u : 0u)
         | (value >= -3 ? 2u : 0u);
}

int main(void) {
    unsigned hash = 0;
    static const uint16_t u16_values[] = {0, 1, 65531, 65532, 65533, 65534, 65535};
    static const uint8_t u8_values[] = {0, 1, 253, 254, 255};
    static const int16_t i16_values[] = {-32768, -4, -3, -2, -1, 0, 32767};
    for (unsigned i = 0; i < sizeof(u16_values) / sizeof(u16_values[0]); ++i)
        hash = hash * 33u + classify_u16(u16_values[i]);
    for (unsigned i = 0; i < sizeof(u8_values) / sizeof(u8_values[0]); ++i)
        hash = hash * 33u + classify_u8(u8_values[i]);
    for (unsigned i = 0; i < sizeof(i16_values) / sizeof(i16_values[0]); ++i)
        hash = hash * 33u + classify_i16(i16_values[i]);
    printf("%08x\n", hash);
    return 0;
}
