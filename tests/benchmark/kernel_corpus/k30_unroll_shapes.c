/* Loop-unroller shape corpus (loop_unroll.rs red-team audit).
 *
 * Each kernel is a counted-loop idiom lifted from the golden workloads
 * (zlib-ng adler32 DO16 macro, gzip/zlib crc bit loop, expat name scanning,
 * decompressor countdowns, small fixed-size reductions).  They exercise the
 * decisions the unroller makes -- complete vs partial, stride, direction,
 * exit polarity, unsigned domains -- and are compared against GCC 16.2,
 * Clang 23.1, ICC and ICX via scripts/codegen_oracle.py.
 */
#include <stddef.h>
#include <stdint.h>

/* zlib-ng adler32 DO16: two-accumulator constant-trip reduction. */
uint32_t adler_do16(uint32_t adler, uint32_t sum2, const unsigned char *buf) {
    for (int i = 0; i < 16; i++) { adler += buf[i]; sum2 += adler; }
    return (sum2 << 16) | (adler & 0xffff);
}

/* Countdown copy: `while (len--)` decompressor idiom, runtime trip. */
void copy_down(unsigned char *dst, const unsigned char *src, unsigned len) {
    for (unsigned i = len; i > 0; i -= 1) *dst++ = *src++;
}

/* Sub-stride countdown, constant trip (i -= 2). */
int sum_odd_down(const int *a) {
    int s = 0;
    for (int i = 15; i >= 1; i -= 2) s += a[i];
    return s;
}

/* Runtime-trip reduction: the partial unroller's bread and butter. */
long sum_n(const int *a, int n) {
    long s = 0;
    for (int i = 0; i < n; i++) s += a[i];
    return s;
}

/* `!=` exit with exact stride: 8-byte lanes. */
uint64_t xor_lanes(const uint64_t *p) {
    uint64_t x = 0;
    for (unsigned i = 0; i != 32; i += 4) x ^= p[i] + p[i + 1] + p[i + 2] + p[i + 3];
    return x;
}

/* expat-style scan: early exit inside the body, IV live-out. */
const char *scan_name(const char *p, const char *end) {
    for (; p != end; p++) {
        unsigned char c = (unsigned char)*p;
        if (!((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || c == '_' || (c >= '0' && c <= '9'))) break;
    }
    return p;
}

/* Nested constant trips: 4x4 fixed matrix-vector product. */
void mat4_vec(const float m[16], const float v[4], float out[4]) {
    for (int r = 0; r < 4; r++) {
        float acc = 0.0f;
        for (int c = 0; c < 4; c++) acc += m[r * 4 + c] * v[c];
        out[r] = acc;
    }
}

/* Polarity: for(;;) { if (guard) break; } with unsigned domain. */
uint32_t pack_bytes(const unsigned char *b) {
    uint32_t v = 0;
    for (unsigned i = 0;; i++) {
        if (i >= 4u) break;
        v |= (uint32_t)b[i] << (8 * i);
    }
    return v;
}

/* Mirrored operand order: limit on the left. */
int sum_mirror(const int *a) {
    int s = 0;
    for (int i = 0; 8 > i; i++) s += a[i] * (i + 1);
    return s;
}

/* Triangular nest: inner init depends on the outer IV (affine const after
 * the outer complete unroll). */
int tri_pairs(const int *a) {
    int s = 0;
    for (int i = 0; i < 5; i++)
        for (int j = i + 1; j < 5; j++) s += a[i] * a[j];
    return s;
}
