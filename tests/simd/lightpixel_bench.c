/* External-call microbenchmark driver, always built by the reference compiler.
 * No LTO: the tested compiler owns only lightpixel_kernels.c. */
#define _POSIX_C_SOURCE 200809L
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
void lightpixel_separate(const unsigned char *, unsigned char *, uint32_t);
uint32_t lightpixel_value(uint32_t, uint32_t);
void lightpixel_wide4(const uint32_t *, uint32_t *, uint32_t);
void lightpixel_wide8(const uint32_t *, uint32_t *, uint32_t);
void umin4(const uint32_t *, const uint32_t *, uint32_t *);
void umax4(const uint32_t *, const uint32_t *, uint32_t *);
void umin8(const uint32_t *, const uint32_t *, uint32_t *);
void umax8(const uint32_t *, const uint32_t *, uint32_t *);
static uint32_t a[2048], b[2048], out[2048];
static unsigned char rgb[256][3], bgra[256][4];
static uint64_t now(void) {
    struct timespec t;
    if (clock_gettime(CLOCK_MONOTONIC, &t) != 0) { perror("clock_gettime"); exit(70); }
    return (uint64_t)t.tv_sec * 1000000000ULL + (uint64_t)t.tv_nsec;
}
int main(int argc, char **argv) {
    if (argc != 2 || argv[1][0] < '0' || argv[1][0] > '7' || argv[1][1]) return 2;
    int kernel = argv[1][0] - '0';
    uint32_t state = 0x63122730U;
    for (int i = 0; i < 2048; ++i) {
        state ^= state << 13; state ^= state >> 17; state ^= state << 5;
        a[i] = state; b[i] = ~state ^ (uint32_t)i;
    }
    for (int i = 0; i < 256; ++i)
        for (int j = 0; j < 3; ++j) rgb[i][j] = (unsigned char)a[i * 3 + j];
    uint32_t sum = 0;
    uint64_t start = now();
    /* Dispatch outside the timed inner loops. Identical call overhead and
     * input generation in every compiler arm; cache-resident data. */
#define REPEAT for (uint32_t i = 0; i < 2097152; ++i)
#define OFFSET ((i & 255U) * 8U)
    switch (kernel) {
    case 0: REPEAT lightpixel_separate(rgb[i & 255U], bgra[i & 255U], i * 2654435761U); break;
    case 1: REPEAT sum += lightpixel_value(a[i & 2047U], i * 2654435761U); break;
    case 2: REPEAT lightpixel_wide4(a + OFFSET, out + OFFSET, i * 2654435761U); break;
    case 3: REPEAT umin4(a + OFFSET, b + OFFSET, out + OFFSET); break;
    case 4: REPEAT umax4(a + OFFSET, b + OFFSET, out + OFFSET); break;
    case 5: REPEAT lightpixel_wide8(a + OFFSET, out + OFFSET, i * 2654435761U); break;
    case 6: REPEAT umin8(a + OFFSET, b + OFFSET, out + OFFSET); break;
    case 7: REPEAT umax8(a + OFFSET, b + OFFSET, out + OFFSET); break;
    }
    uint64_t elapsed = now() - start;
    uint64_t hash = sum;
    for (unsigned i = 0; i < 2048; ++i) hash = (hash ^ out[i]) * 1099511628211ULL;
    for (unsigned i = 0; i < 256; ++i)
        for (unsigned j = 0; j < 4; ++j) hash = (hash ^ bgra[i][j]) * 1099511628211ULL;
    printf("%d %016llx %llu\n", kernel, (unsigned long long)hash, (unsigned long long)elapsed);
    return 0;
}
