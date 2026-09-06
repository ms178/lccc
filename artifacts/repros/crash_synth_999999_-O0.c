/* Synthetic differential test program */
/* Seed: 999999 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

static volatile int16_t g_0 = 27584;
static volatile uint8_t g_1 = 57213;
static volatile uint8_t g_2 = 49583;
static volatile int64_t g_3 = 8326;
static volatile int64_t g_4 = 21041;

struct S0 {
    int32_t a;
    uint16_t b;
    int64_t c;
    uint8_t d[4];
};
static struct S0 g_s0 = { 1234, 56, 789012345678ULL, { 1, 2, 3, 4 } };

static int64_t func_0(int64_t a, int64_t b, int32_t c) {
    int64_t acc = a ^ (b << 3);
    int32_t arr[8];
    for (int i = 0; i < 8; i++) arr[i] = (int32_t)(c + i * 17);
    for (int i = 0; i < 4; i++) {
        acc += arr[i] * (arr[7 - i] + 1);
        acc = (acc >> 1) ^ (acc << 5);
    }
    g_0 += (int32_t)acc;
    return acc ^ g_s0.c;
}

static int64_t func_1(int64_t a, int64_t b, int32_t c) {
    int64_t acc = a ^ (b << 3);
    int32_t arr[8];
    for (int i = 0; i < 8; i++) arr[i] = (int32_t)(c + i * 17);
    for (int i = 0; i < 4; i++) {
        acc += arr[i] * (arr[7 - i] + 1);
        acc = (acc >> 1) ^ (acc << 5);
    }
    g_1 += (int32_t)acc;
    return acc ^ g_s0.c;
}

static int64_t func_2(int64_t a, int64_t b, int32_t c) {
    int64_t acc = a ^ (b << 3);
    int32_t arr[8];
    for (int i = 0; i < 8; i++) arr[i] = (int32_t)(c + i * 17);
    for (int i = 0; i < 4; i++) {
        acc += arr[i] * (arr[7 - i] + 1);
        acc = (acc >> 1) ^ (acc << 5);
    }
    g_2 += (int32_t)acc;
    return acc ^ g_s0.c;
}

static uint64_t crc = 0x123456789ABCDEF0ULL;
static void crc_update(uint64_t val) {
    crc = (crc ^ val) * 0x100000001B3ULL + 0xCBF29CE484222325ULL;
}

int main(void) {
    int64_t x = 42, y = 1337;
    for (int iter = 0; iter < 20; iter++) {
        crc_update((uint64_t)func_0(x, y, (int32_t)iter));
        crc_update((uint64_t)func_1(x, y, (int32_t)iter));
        crc_update((uint64_t)func_2(x, y, (int32_t)iter));
        x = (int64_t)(crc & 0xFFFFFF);
        y = (int64_t)(g_s0.a + (int32_t)iter);
    }
    crc_update((uint64_t)g_s0.a);
    crc_update((uint64_t)g_s0.b);
    crc_update((uint64_t)g_s0.c);
    for (int i = 0; i < 4; i++) crc_update((uint64_t)g_s0.d[i]);
    printf("CRC: %016llx\n", (unsigned long long)crc);
    return 0;
}
