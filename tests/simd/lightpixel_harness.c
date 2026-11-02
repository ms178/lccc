#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>
void lightpixel(unsigned char *, uint32_t);
void lightpixel_separate(const unsigned char *, unsigned char *, uint32_t);
uint32_t lightpixel_value(uint32_t, uint32_t);
void lightpixel_wide4(const uint32_t *, uint32_t *, uint32_t);
void umin4(const uint32_t *, const uint32_t *, uint32_t *);
void umax4(const uint32_t *, const uint32_t *, uint32_t *);
void lightpixel_scattered(const unsigned char *, unsigned char *, uint32_t);
void lightpixel_volatile(volatile unsigned char *, uint32_t);
void lightpixel_interference(unsigned char *, unsigned char *, uint32_t);
void lightpixel_wide8(const uint32_t *, uint32_t *, uint32_t);
void umin8(const uint32_t *, const uint32_t *, uint32_t *);
void umax8(const uint32_t *, const uint32_t *, uint32_t *);
static uint32_t state = 0x63122730U;
static uint32_t random32(void) {
    state ^= state << 13; state ^= state >> 17; state ^= state << 5;
    return state;
}
static unsigned char ref_channel(unsigned char c, uint32_t s) {
    /* Use a wider multiply and an explicit low-32-bit mask independently
     * of the candidate's uint32 multiply. */
    uint32_t v = (uint32_t)(((uint64_t)c * s) & UINT32_MAX) >> 15;
    return v > 255 ? 255 : (unsigned char)v;
}
static void reference(unsigned char *p, uint32_t s) {
    unsigned char a = 0;
    for (int j = 0; j < 3; ++j) {
        p[j] = ref_channel(p[j], s);
        if (p[j] > a) a = p[j];
    }
    p[3] = a;
}
static void check(int ok, const char *what, unsigned n) {
    if (!ok) { fprintf(stderr, "%s case %u\n", what, n); exit(1); }
}
static unsigned char *guarded(size_t n, int end) {
    long page = sysconf(_SC_PAGESIZE);
    check(page > 0 && n <= (size_t)page, "page size", 0);
    unsigned char *p = mmap(0, (size_t)page * 3, PROT_NONE,
                            MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check(p != MAP_FAILED, "mmap", 0);
    check(mprotect(p + page, (size_t)page, PROT_READ | PROT_WRITE) == 0,
          "mprotect", 0);
    return p + page + (end ? (size_t)page - n : 0);
}
int main(void) {
    static const uint32_t scales[] = {0, 1, 32767, 32768, 32769, 65535,
        65536, 0x7fffffffU, 0x80000000U, 0x80000001U, 0xfffffffeU,
        0xffffffffU, 0x01010101U, 8388607, 8388608, 16843010};
    unsigned char *inputs[2] = {guarded(3, 0), guarded(3, 1)};
    unsigned char *outputs[2] = {guarded(4, 0), guarded(4, 1)};
    for (unsigned n = 0; n < 65536; ++n) {
        uint32_t color = random32();
        uint32_t scale = n < 4096 ? scales[n / 256] : random32();
        if (n < 4096) {
            uint32_t x = n % 256;
            color = (color & 0xff000000U) | x | ((255 - x) << 8)
                    | (((x * 173) & 255) << 16);
        }
        unsigned char original[4] = {color, color >> 8, color >> 16, color >> 24};
        unsigned char expected[4], actual[4];
        memcpy(expected, original, 4); reference(expected, scale);
        memcpy(actual, original, 4); lightpixel(actual, scale);
        check(memcmp(actual, expected, 4) == 0, "inplace", n);
        memcpy(actual, original, 4); lightpixel_volatile(actual, scale);
        check(memcmp(actual, expected, 4) == 0, "volatile", n);
        uint32_t packed = (uint32_t)expected[0] | ((uint32_t)expected[1] << 8)
                        | ((uint32_t)expected[2] << 16) | ((uint32_t)expected[3] << 24);
        check(lightpixel_value(color, scale) == packed, "value", n);
        for (int edge = 0; edge < 2; ++edge) {
            memcpy(inputs[edge], original, 3);
            memset(outputs[edge], 0xa5, 4);
            lightpixel_separate(inputs[edge], outputs[edge], scale);
            check(memcmp(outputs[edge], expected, 4) == 0, "guarded separate", n);
            check(memcmp(inputs[edge], original, 3) == 0, "const input", n);
            memcpy(outputs[edge], original, 4);
            lightpixel(outputs[edge], scale);
            check(memcmp(outputs[edge], expected, 4) == 0, "guarded inplace", n);
        }
        for (int delta = -3; delta <= 3; ++delta) {
            unsigned char overlap[16], ref[16];
            memset(overlap, 0xa5, sizeof(overlap));
            memcpy(overlap + 6, original, 3);
            memcpy(ref, overlap, sizeof(ref));
            memcpy(ref + 6 + delta, expected, 4);
            lightpixel_separate(overlap + 6, overlap + 6 + delta, scale);
            check(memcmp(overlap, ref, sizeof(ref)) == 0, "separate overlap", n);
        }
        unsigned char scattered[9]; memset(scattered, 0xa5, sizeof(scattered));
        lightpixel_scattered(original, scattered + 1, scale);
        for (int j = 0; j < 9; ++j)
            check(scattered[j] == (j % 2 ? expected[j / 2] : 0xa5), "scattered", n);
        for (int alias = 0; alias < 4; ++alias) {
            unsigned char ref[4]; memcpy(ref, original, 4); memcpy(actual, original, 4);
            ref[0] = ref_channel(ref[0], scale); ref[alias] = 173;
            ref[1] = ref_channel(ref[1], scale); ref[2] = ref_channel(ref[2], scale);
            ref[3] = ref[0] > ref[1] ? ref[0] : ref[1];
            if (ref[2] > ref[3]) ref[3] = ref[2];
            lightpixel_interference(actual, actual + alias, scale);
            check(memcmp(actual, ref, 4) == 0, "interference", n);
        }
        uint32_t a[8], b[8], got[8];
        for (int j = 0; j < 8; ++j) { a[j] = random32(); b[j] = random32(); }
        /* Systematically straddle the signed/unsigned ordering boundary. */
        a[n % 4] = n % 2 ? 0x80000000U : 0xffffffffU;
        b[n % 4] = n % 2 ? 0x7fffffffU : 0;
        umin4(a, b, got);
        for (int j = 0; j < 4; ++j) check(got[j] == (a[j] < b[j] ? a[j] : b[j]), "umin", n);
        umax4(a, b, got);
        for (int j = 0; j < 4; ++j) check(got[j] == (a[j] > b[j] ? a[j] : b[j]), "umax", n);
        lightpixel_wide4(a, got, scale);
        for (int j = 0; j < 4; ++j) {
            uint32_t v = (uint32_t)((uint64_t)a[j] * scale) >> 15;
            check(got[j] == (v < 255U ? v : 255U), "wide clamp", n);
        }
        umin8(a, b, got);
        for (int j = 0; j < 8; ++j) check(got[j] == (a[j] < b[j] ? a[j] : b[j]), "umin8", n);
        umax8(a, b, got);
        for (int j = 0; j < 8; ++j) check(got[j] == (a[j] > b[j] ? a[j] : b[j]), "umax8", n);
        lightpixel_wide8(a, got, scale);
        for (int j = 0; j < 8; ++j) {
            uint32_t v = (uint32_t)((uint64_t)a[j] * scale) >> 15;
            check(got[j] == (v < 255U ? v : 255U), "wide clamp8", n);
        }

    }
    puts("PASS lightpixel 65536 cases 12 kernels guarded3/4 alias0..3 full-u32");
    return 0;
}
