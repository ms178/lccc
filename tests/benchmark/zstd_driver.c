/* Benchmark infrastructure, built once with GCC for every library arm.
 * Timed region contains only repeated public zstd API calls on preloaded data.
 * A common reference frame is decoded by all arms. Every result is checked.
 */
#include <zstd.h>
#include <valgrind/callgrind.h>
#include <errno.h>
#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

static void fail(const char *message) { fprintf(stderr, "%s\n", message); exit(1); }
static uint64_t now(void) {
    struct timespec t;
    if (clock_gettime(CLOCK_MONOTONIC, &t)) fail("clock failed");
    return (uint64_t)t.tv_sec * UINT64_C(1000000000) + (uint64_t)t.tv_nsec;
}
static void *read_file(const char *path, size_t *size) {
    FILE *f = fopen(path, "rb");
    if (!f || fseek(f, 0, SEEK_END)) fail("input open/seek failed");
    long length = ftell(f);
    if (length <= 0 || fseek(f, 0, SEEK_SET)) fail("invalid input length");
    *size = (size_t)length;
    void *p = malloc(*size);
    if (!p || fread(p, 1, *size, f) != *size || fclose(f)) fail("input read failed");
    return p;
}
static void check(size_t result) { if (ZSTD_isError(result)) fail(ZSTD_getErrorName(result)); }
static uint64_t parse(const char *s) {
    char *end; errno = 0;
    unsigned long long n = strtoull(s, &end, 10);
    if (errno || !*s || *end || !n || *s == '-') fail("invalid positive integer");
    return n;
}
int main(int argc, char **argv) {
    if (argc != 7) fail("usage: input reference-frame output level c|d iterations");
    size_t input_size, frame_size = 0;
    unsigned char *input = read_file(argv[1], &input_size);
    unsigned char *frame = NULL;
    size_t capacity = ZSTD_compressBound(input_size);
    unsigned char *compressed = malloc(capacity), *decoded = malloc(input_size);
    if (!compressed || !decoded) fail("allocation failed");
    uint64_t raw_level = parse(argv[4]);
    if (raw_level > (uint64_t)ZSTD_maxCLevel()) fail("unsupported level");
    int level = (int)raw_level;
    int decompress = !strcmp(argv[5], "d");
    if (!decompress && strcmp(argv[5], "c")) fail("invalid operation");
    uint64_t iterations = parse(argv[6]);
    ZSTD_CCtx *cc = ZSTD_createCCtx();
    ZSTD_DCtx *dc = ZSTD_createDCtx();
    if (!cc || !dc) fail("context allocation failed");
    if (decompress) frame = read_file(argv[2], &frame_size);
    /* Excluded warm-up initializes contexts and verifies the input/frame pair. */
    size_t result;
    if (decompress) {
        result = ZSTD_decompressDCtx(dc, decoded, input_size, frame, frame_size);
        check(result);
        if (result != input_size || memcmp(input, decoded, input_size)) fail("reference frame mismatch");
    } else {
        result = ZSTD_compressCCtx(cc, compressed, capacity, input, input_size, level);
        check(result);
    }
    uint64_t begin = now();
    CALLGRIND_START_INSTRUMENTATION;
    CALLGRIND_ZERO_STATS;
    for (uint64_t i = 0; i < iterations; ++i) {
        result = decompress
            ? ZSTD_decompressDCtx(dc, decoded, input_size, frame, frame_size)
            : ZSTD_compressCCtx(cc, compressed, capacity, input, input_size, level);
        check(result);
    }
    CALLGRIND_STOP_INSTRUMENTATION;
    uint64_t elapsed = now() - begin;
    size_t decoded_size = result;
    if (!decompress) {
        decoded_size = ZSTD_decompressDCtx(dc, decoded, input_size, compressed, result);
        check(decoded_size);
    }
    if (decoded_size != input_size || memcmp(input, decoded, input_size)) fail("round-trip mismatch");
    if (strcmp(argv[3], "-")) {
        FILE *f = fopen(argv[3], "wb");
        const void *data = decompress ? (const void *)decoded : (const void *)compressed;
        size_t n = decompress ? input_size : result;
        if (!f || fwrite(data, 1, n, f) != n || fclose(f)) fail("output write failed");
    }
    printf("{\"ns\":%" PRIu64 ",\"iterations\":%" PRIu64 ",\"input_bytes\":%zu,\"result_bytes\":%zu,\"zstd_version\":%u}\n",
           elapsed, iterations, input_size, result, ZSTD_versionNumber());
    ZSTD_freeCCtx(cc); ZSTD_freeDCtx(dc);
    free(input); free(frame); free(compressed); free(decoded);
    return 0;
}
