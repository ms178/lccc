/*
 * Outer/inner perfect-nest shapes: the flattening/vectorization gap corpus.
 *
 * GCC 16.2 and Clang 23.1 flatten and vectorize s1/s2/s4/s5/s6 at
 * -O2 -march=x86-64-v3 (GCC additionally misses s3, Clang needs runtime
 * alias checks there); lccc's vectorizer only processes innermost loops,
 * and loop_unroll fully unrolls the constant-trip inner loop first, so
 * the nest stays scalar (engineering/FOLLOWUP-2026-09-08, section 2).
 *
 * This file is the correctness oracle for the upcoming loop flattener:
 * every kernel's output is folded into one FNV-1a hash. The flattener
 * must keep this byte-identical to the scalar execution (and to GCC)
 * at every optimization level and array size class exercised here,
 * including the vector/remainder boundary sizes.
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define N 512

static uint32_t hb(const void *p, size_t n) {
    const uint8_t *b = (const uint8_t *) p;
    uint32_t h = 2166136261u;
    for (size_t i = 0; i < n; i++)
        h = (h ^ b[i]) * 16777619u;
    return h;
}

/* s1: perfect nest, constant inner trip 4 == AVX2 i32 width, map body. */
static void s1_map4(int *restrict dst, const int *restrict src, int n) {
    for (int i = 0; i < n; i++)
        for (int j = 0; j < 4; j++)
            dst[i * 4 + j] = src[i * 4 + j] * 3 + 1;
}

/* s2: f32, constant inner trip 8 == AVX2 f32 width. */
static void s2_fmap8(float *restrict dst, const float *restrict src, float k, int n) {
    for (int i = 0; i < n; i++)
        for (int j = 0; j < 8; j++)
            dst[i * 8 + j] = src[i * 8 + j] * k;
}

/* s3: row-pointer form — GCC MISSES this one; the flattener's
 * differentiation target (pointer phi advancing by a constant). */
static void s3_rows(int n, int (*dst)[4], const int (*src)[4]) {
    for (int i = 0; i < n; i++)
        for (int j = 0; j < 4; j++)
            dst[i][j] = src[i][j] + 7;
}

/* s4: inner reduction with per-row reset — NOT a map; the flattened form
 * must preserve the reduction semantics exactly. */
static void s4_rowsum(const int *restrict a, int *restrict out, int n) {
    for (int i = 0; i < n; i++) {
        int s = 0;
        for (int j = 0; j < 8; j++)
            s += a[i * 8 + j];
        out[i] = s;
    }
}

/* s5: i64 elements, constant inner trip 4. */
static void s5_i64(int64_t *restrict d, const int64_t *restrict s, int n) {
    for (int i = 0; i < n; i++)
        for (int j = 0; j < 4; j++)
            d[i * 4 + j] = s[i * 4 + j] | 1;
}

/* s6: i16 elements, constant inner trip 4. */
static void s6_i16(int16_t *restrict d, const int16_t *restrict s, int n) {
    for (int i = 0; i < n; i++)
        for (int j = 0; j < 4; j++)
            d[i * 4 + j] = (int16_t) (s[i * 4 + j] + 4);
}

/* s7: plain counted loop (flattening must not touch it). */
static void s7_axpy(float *restrict y, const float *restrict x, float a, int n) {
    for (int i = 0; i < n; i++)
        y[i] = y[i] * a + x[i];
}

/* s8: stride-2 in j — the flattener's affine proof must REJECT this
 * (non-unit inner stride is not `C*iv_o + iv_i` with the vectorizer's
 * unit-stride addressing). Staying scalar here is correct. */
static void s8_stride2(int *restrict d, const int *restrict s, int n) {
    for (int i = 0; i < n; i++)
        for (int j = 0; j < 4; j++)
            d[i * 8 + 2 * j] = s[i * 8 + 2 * j] + 5;
}

static int A4[N * 4], B4[N * 4], C4[N * 4];
static float AF8[N * 8], BF8[N * 8], K = 1.5f;
static int R3d[N][4], S3d[N][4];
static int A8[N * 8], OUT8[N];
static int64_t A64[N * 4], B64[N * 4];
static int16_t A16[N * 4], B16[N * 4];
static float AY[N], AX[N];
static int AST[N * 8], BST[N * 8];

int main(void) {
    uint32_t h = 2166136261u;
    for (int i = 0; i < N * 4; i++) {
        B4[i] = i * 7 - 3;
        A64[i] = i * 11 + 5;
        B16[i] = (int16_t) (i * 13);
    }
    for (int i = 0; i < N * 8; i++) {
        BF8[i] = (float) i * 0.25f;
        A8[i] = (i * 31) & 0xff;
        AST[i] = (i * 17) & 0x3f;
        BST[i] = (i * 5) & 0x7f;
    }
    for (int i = 0; i < N; i++) {
        for (int j = 0; j < 4; j++) {
            S3d[i][j] = i * 4 + j;
        }
        AY[i] = (float) i * 0.5f;
        AX[i] = (float) i * 0.125f;
    }
    memset(A4, 0, sizeof A4);
    memset(AF8, 0, sizeof AF8);
    memset(R3d, 0, sizeof R3d);
    memset(OUT8, 0, sizeof OUT8);
    memset(A64, 0, sizeof A64);
    memset(A16, 0, sizeof A16);
    memset(AY, 0, sizeof AY);

    /* Full size + boundary classes: n=0 (no-op), n=1, and odd n. */
    int sizes[] = {N, 0, 1, 37};
    for (size_t k = 0; k < sizeof sizes / sizeof sizes[0]; k++) {
        int n = sizes[k];
        s1_map4(A4, B4, n);
        s2_fmap8(AF8, BF8, K, n);
        s3_rows(n, R3d, S3d);
        s4_rowsum(A8, OUT8, n);
        s5_i64(A64, B64, n);
        s6_i16(A16, B16, n);
        s7_axpy(AY, AX, K, n);
        s8_stride2(AST, BST, n);
    }
    /* Re-run on the same buffers (in-place read-after-write across calls
     * catches aliasing mistakes a fresh-buffer run cannot). */
    s1_map4(A4, A4, N / 2);
    s3_rows(N / 2, R3d, (const int (*)[4]) R3d);
    s5_i64(A64, A64, N / 2);

    h = hb(A4, sizeof A4);
    printf("s1 %08x\n", h);
    h = (h ^ hb(AF8, sizeof AF8)) * 16777619u;
    printf("s2 %08x\n", hb(AF8, sizeof AF8));
    printf("s3 %08x\n", hb(R3d, sizeof R3d));
    printf("s4 %08x\n", hb(OUT8, sizeof OUT8));
    printf("s5 %08x\n", hb(A64, sizeof A64));
    printf("s6 %08x\n", hb(A16, sizeof A16));
    printf("s7 %08x\n", hb(AY, sizeof AY));
    printf("s8 %08x\n", hb(AST, sizeof AST));
    return 0;
}
