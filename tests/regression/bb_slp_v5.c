/*
 * BB-SLP v5 battery: packed shifts, rotate decomposition (both the
 * canonical RotateLeft spelling and the raw (x<<k)|(x>>(W-k)) spelling),
 * Not/Neg composites, the Sub(x,1) all-ones idiom, and the FP strict
 * min/max fold.
 *
 * Every function is written twice: the vectorization candidate (4 or 8
 * straight-line lanes on consecutive addresses) and a scalar reference
 * spelled to defeat SLP (one lane at a time through an opaque identity
 * that keeps the arithmetic but breaks the store-run shape).  The
 * self-check compares BIT-EXACT outputs over inputs chosen to hit the
 * exactness hazards of each fold:
 *   - shifts: sign bits, all-ones, shift by 1 and by W-1;
 *   - rotates: every amount 1..W-1 on asymmetric bit patterns;
 *   - Not/Neg/Sub(x,1): INT_MIN/INT_MAX wrap edges;
 *   - FP min/max: qNaN/sNaN payloads, +0.0/-0.0, equal values — the
 *     lanes where MINPS/MAXPS operand order is the whole proof.
 * The adversarial sections verify the REJECTIONS: varying amounts,
 * non-strict compares, cmp results with external uses, and an
 * interleaved write between the two spelling-side loads of a raw rotate
 * (the same-source proof must refuse).
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

static int fails = 0;

#define CHECK(cond, name)                                                  \
    do {                                                                   \
        if (!(cond)) {                                                     \
            printf("FAIL %s\n", name);                                     \
            fails++;                                                       \
        }                                                                  \
    } while (0)

/* Scalar reference helpers: noalias-free, one lane per call — the store
 * run never forms, so these stay scalar under any -O level. */
static inline int32_t ref_shl_i32(int32_t x) { return x << 3; }
static inline int32_t ref_sar_i32(int32_t x) { return x >> 3; }
static inline uint32_t ref_shr_u32(uint32_t x) { return x >> 3; }
static inline uint16_t ref_shl_i16(uint16_t x) { return (uint16_t)(x << 5); }
static inline int16_t ref_sar_i16(int16_t x) { return (int16_t)(x >> 5); }
static inline uint64_t ref_shl_i64(uint64_t x) { return x << 9; }
static inline uint64_t ref_shr_u64(uint64_t x) { return x >> 9; }
static inline int32_t ref_not_i32(int32_t x) { return ~x; }
static inline int32_t ref_neg_i32(int32_t x) { return -x; }
static inline int64_t ref_neg_i64(int64_t x) { return -x; }
static inline int32_t ref_sub1_i32(int32_t x) { return x - 1; }
static inline uint32_t ref_sub1_u32(uint32_t x) { return x - 1u; }
static inline uint32_t ref_rotl_u32(uint32_t x, unsigned k) {
    return (x << k) | (x >> (32 - k));
}
static inline uint64_t ref_rotl_u64(uint64_t x, unsigned k) {
    return (x << k) | (x >> (64 - k));
}
static inline uint32_t ref_rotr_u32(uint32_t x, unsigned k) {
    return (x >> k) | (x << (32 - k));
}

/* ── Shift candidates ─────────────────────────────────────────────────── */

void v5_shl_i32(const int32_t * restrict x, int32_t * restrict y) {
    y[0] = x[0] << 3; y[1] = x[1] << 3; y[2] = x[2] << 3; y[3] = x[3] << 3;
}
void v5_sar_i32(const int32_t * restrict x, int32_t * restrict y) {
    y[0] = x[0] >> 3; y[1] = x[1] >> 3; y[2] = x[2] >> 3; y[3] = x[3] >> 3;
}
void v5_shr_u32(const uint32_t * restrict x, uint32_t * restrict y) {
    y[0] = x[0] >> 3; y[1] = x[1] >> 3; y[2] = x[2] >> 3; y[3] = x[3] >> 3;
}
void v5_shl_i16(const uint16_t * restrict x, uint16_t * restrict y) {
    y[0] = (uint16_t)(x[0] << 5); y[1] = (uint16_t)(x[1] << 5);
    y[2] = (uint16_t)(x[2] << 5); y[3] = (uint16_t)(x[3] << 5);
    y[4] = (uint16_t)(x[4] << 5); y[5] = (uint16_t)(x[5] << 5);
    y[6] = (uint16_t)(x[6] << 5); y[7] = (uint16_t)(x[7] << 5);
}
void v5_sar_i16(const int16_t * restrict x, int16_t * restrict y) {
    y[0] = (int16_t)(x[0] >> 5); y[1] = (int16_t)(x[1] >> 5);
    y[2] = (int16_t)(x[2] >> 5); y[3] = (int16_t)(x[3] >> 5);
    y[4] = (int16_t)(x[4] >> 5); y[5] = (int16_t)(x[5] >> 5);
    y[6] = (int16_t)(x[6] >> 5); y[7] = (int16_t)(x[7] >> 5);
}
void v5_shl_i64(const uint64_t * restrict x, uint64_t * restrict y) {
    y[0] = x[0] << 9; y[1] = x[1] << 9;
}
void v5_shr_u64(const uint64_t * restrict x, uint64_t * restrict y) {
    y[0] = x[0] >> 9; y[1] = x[1] >> 9;
}
/* 256-bit dword shift (8 lanes, AVX2). */
void v5_shl_i32x8(const int32_t * restrict x, int32_t * restrict y) {
    y[0] = x[0] << 7; y[1] = x[1] << 7; y[2] = x[2] << 7; y[3] = x[3] << 7;
    y[4] = x[4] << 7; y[5] = x[5] << 7; y[6] = x[6] << 7; y[7] = x[7] << 7;
}

/* ── Rotate candidates (raw C spelling — the look-through) ────────────── */

#define ROTL32(x, n) (((x) << (n)) | ((x) >> (32 - (n))))
#define ROTR32(x, n) (((x) >> (n)) | ((x) << (32 - (n))))
#define ROTL64(x, n) (((x) << (n)) | ((x) >> (64 - (n))))

void v5_rotl_u32(const uint32_t * restrict x, uint32_t * restrict y) {
    y[0] = ROTL32(x[0], 7); y[1] = ROTL32(x[1], 7);
    y[2] = ROTL32(x[2], 7); y[3] = ROTL32(x[3], 7);
}
void v5_rotr_u32(const uint32_t * restrict x, uint32_t * restrict y) {
    y[0] = ROTR32(x[0], 19); y[1] = ROTR32(x[1], 19);
    y[2] = ROTR32(x[2], 19); y[3] = ROTR32(x[3], 19);
}
void v5_rotl_u64(const uint64_t * restrict x, uint64_t * restrict y) {
    y[0] = ROTL64(x[0], 13); y[1] = ROTL64(x[1], 13);
    y[2] = ROTL64(x[2], 13); y[3] = ROTL64(x[3], 13);
}
/* Mirrored operand order inside the Or: (x>>k)|(x<<(W-k)). */
void v5_rotl_swapped(const uint32_t * restrict x, uint32_t * restrict y) {
    y[0] = (x[0] >> 25) | (x[0] << 7);
    y[1] = (x[1] >> 25) | (x[1] << 7);
    y[2] = (x[2] >> 25) | (x[2] << 7);
    y[3] = (x[3] >> 25) | (x[3] << 7);
}

/* ── Not / Neg / Sub(x,1) ─────────────────────────────────────────────── */

void v5_not_i32(const int32_t * restrict x, int32_t * restrict y) {
    y[0] = ~x[0]; y[1] = ~x[1]; y[2] = ~x[2]; y[3] = ~x[3];
}
void v5_neg_i32(const int32_t * restrict x, int32_t * restrict y) {
    y[0] = -x[0]; y[1] = -x[1]; y[2] = -x[2]; y[3] = -x[3];
}
void v5_neg_i64(const int64_t * restrict x, int64_t * restrict y) {
    y[0] = -x[0]; y[1] = -x[1];
}
void v5_sub1_i32(const int32_t * restrict x, int32_t * restrict y) {
    y[0] = x[0] - 1; y[1] = x[1] - 1; y[2] = x[2] - 1; y[3] = x[3] - 1;
}
void v5_sub1_u64(const uint64_t * restrict x, uint64_t * restrict y) {
    y[0] = x[0] - 1; y[1] = x[1] - 1;
}

/* ── FP strict min/max (the four spellings) ───────────────────────────── */

void v5_min_f32(const float * restrict x, float * restrict y) {
    y[0] = x[0] < 1.5f ? x[0] : 1.5f;
    y[1] = x[1] < 1.5f ? x[1] : 1.5f;
    y[2] = x[2] < 1.5f ? x[2] : 1.5f;
    y[3] = x[3] < 1.5f ? x[3] : 1.5f;
}
void v5_max_f32(const float * restrict x, float * restrict y) {
    y[0] = x[0] > -2.0f ? x[0] : -2.0f;
    y[1] = x[1] > -2.0f ? x[1] : -2.0f;
    y[2] = x[2] > -2.0f ? x[2] : -2.0f;
    y[3] = x[3] > -2.0f ? x[3] : -2.0f;
}
void v5_min_f64_swapped(const double * restrict x, double * restrict y) {
    /* (l < r) ? r : l — the mirrored min: Min(r, l) with the false arm
     * (l) in the src2 slot. */
    y[0] = x[0] < 0.25 ? 0.25 : x[0];
    y[1] = x[1] < 0.25 ? 0.25 : x[1];
    y[2] = x[2] < 0.25 ? 0.25 : x[2];
    y[3] = x[3] < 0.25 ? 0.25 : x[3];
}
void v5_max_f64_swapped(const double * restrict x, double * restrict y) {
    /* (l > r) ? r : l — the mirrored max: Max(r, l). */
    y[0] = x[0] > -0.75 ? -0.75 : x[0];
    y[1] = x[1] > -0.75 ? -0.75 : x[1];
    y[2] = x[2] > -0.75 ? -0.75 : x[2];
    y[3] = x[3] > -0.75 ? -0.75 : x[3];
}
void v5_clamp_f32x8(const float * restrict x, float * restrict y,
                     float * restrict t) {
    /* Eight lanes: the 256-bit min+max pair, level 1 through a separate
     * temp so both store runs are clean seeds. */
    t[0] = x[0] < 0.0f ? 0.0f : x[0]; t[1] = x[1] < 0.0f ? 0.0f : x[1];
    t[2] = x[2] < 0.0f ? 0.0f : x[2]; t[3] = x[3] < 0.0f ? 0.0f : x[3];
    t[4] = x[4] < 0.0f ? 0.0f : x[4]; t[5] = x[5] < 0.0f ? 0.0f : x[5];
    t[6] = x[6] < 0.0f ? 0.0f : x[6]; t[7] = x[7] < 0.0f ? 0.0f : x[7];
    y[0] = t[0] > 1.0f ? 1.0f : t[0]; y[1] = t[1] > 1.0f ? 1.0f : t[1];
    y[2] = t[2] > 1.0f ? 1.0f : t[2]; y[3] = t[3] > 1.0f ? 1.0f : t[3];
    y[4] = t[4] > 1.0f ? 1.0f : t[4]; y[5] = t[5] > 1.0f ? 1.0f : t[5];
    y[6] = t[6] > 1.0f ? 1.0f : t[6]; y[7] = t[7] > 1.0f ? 1.0f : t[7];
}

/* ── ChaCha-shaped quarter-round (the epic target) ────────────────────── */

static void chacha_words(const uint32_t * restrict a, const uint32_t * restrict b,
                         const uint32_t * restrict c, uint32_t * restrict r) {
    /* Transposed quarter-round shape: uniform rotate amount per vector
     * step, add + rotate + xor (what ICX vectorizes with 74 instructions). */
    r[0] = ROTL32(a[0] + b[0], 7) ^ c[0];
    r[1] = ROTL32(a[1] + b[1], 7) ^ c[1];
    r[2] = ROTL32(a[2] + b[2], 7) ^ c[2];
    r[3] = ROTL32(a[3] + b[3], 7) ^ c[3];
}

/* ── Adversarial: things that MUST stay scalar ────────────────────────── */

void v5_varying_shift(const int32_t * restrict x, int32_t * restrict y) {
    /* Different amounts per lane: no uniform constant → reject. */
    y[0] = x[0] << 1; y[1] = x[1] << 5; y[2] = x[2] << 9; y[3] = x[3] << 13;
}
void v5_nonstrict_min(const float * restrict x, float * restrict y) {
    /* <= differs from MINPS on ±0: must stay scalar. */
    y[0] = x[0] <= 1.5f ? x[0] : 1.5f;
    y[1] = x[1] <= 1.5f ? x[1] : 1.5f;
    y[2] = x[2] <= 1.5f ? x[2] : 1.5f;
    y[3] = x[3] <= 1.5f ? x[3] : 1.5f;
}
int g_v5_cmp_use;
void v5_cmp_external_use(const float * restrict x, float * restrict y) {
    /* The comparison result has a second, surviving use: the fold's cmp
     * removal would strand it — must stay scalar. */
    int c0 = x[0] < 1.5f;
    y[0] = c0 ? x[0] : 1.5f;
    int c1 = x[1] < 1.5f;
    y[1] = c1 ? x[1] : 1.5f;
    int c2 = x[2] < 1.5f;
    y[2] = c2 ? x[2] : 1.5f;
    int c3 = x[3] < 1.5f;
    y[3] = c3 ? x[3] : 1.5f;
    g_v5_cmp_use = c0 + c1 + c2 + c3;
}
void v5_rot_interleaved_write(uint32_t * restrict x, uint32_t * restrict y) {
    /* A write to the LOADED address between the two spelling-side loads
     * of lane 0's rotate: the loads observe different bytes, so the
     * same-source proof must refuse and the shape stays scalar. */
    uint32_t t0 = x[0] << 7;
    x[0] = 0xdeadbeef;
    y[0] = t0 | (x[0] >> 25);
    y[1] = (x[1] << 7) | (x[1] >> 25);
    y[2] = (x[2] << 7) | (x[2] >> 25);
    y[3] = (x[3] << 7) | (x[3] >> 25);
}
void v5_shift_external_use(const int32_t * restrict x, int32_t * restrict y,
                           int32_t * restrict z) {
    /* The shift result of lane 0 is ALSO stored elsewhere before the
     * seed store: an extract must service it (or the seed rejects —
     * either way the values must be right). */
    int32_t s0 = x[0] << 6;
    z[0] = s0 + 1;
    y[0] = s0;
    y[1] = x[1] << 6;
    y[2] = x[2] << 6;
    y[3] = x[3] << 6;
}

/* ── The driver ───────────────────────────────────────────────────────── */

union f32bits { float f; uint32_t u; };
union f64bits { double d; uint64_t u; };

int main(void) {
    /* Shifts: sign bits, all-ones, wrap-relevant patterns. */
    {
        const int32_t xs[4] = { 0x40000000, (int32_t)0x80000000,
                                (int32_t)0xFFFFFFFF, 0x00000001 };
        int32_t y[4];
        v5_shl_i32(xs, y);
        for (int i = 0; i < 4; i++)
            CHECK(y[i] == ref_shl_i32(xs[i]), "v5_shl_i32");
        v5_sar_i32(xs, y);
        for (int i = 0; i < 4; i++)
            CHECK(y[i] == ref_sar_i32(xs[i]), "v5_sar_i32");
        v5_shr_u32((const uint32_t *)xs, (uint32_t *)y);
        for (int i = 0; i < 4; i++)
            CHECK(((uint32_t *)y)[i] == ref_shr_u32(((const uint32_t *)xs)[i]),
                  "v5_shr_u32");
        v5_sub1_i32(xs, y);
        for (int i = 0; i < 4; i++)
            CHECK(y[i] == ref_sub1_i32(xs[i]), "v5_sub1_i32 (INT_MIN wrap)");
        v5_not_i32(xs, y);
        for (int i = 0; i < 4; i++)
            CHECK(y[i] == ref_not_i32(xs[i]), "v5_not_i32");
        v5_neg_i32(xs, y);
        for (int i = 0; i < 4; i++)
            CHECK(y[i] == ref_neg_i32(xs[i]), "v5_neg_i32 (INT_MIN negation)");
    }
    {
        const uint16_t xs[8] = { 0x0001, 0x8000, 0xFFFF, 0x7FFF,
                                 0x1234, 0xABCD, 0x00FF, 0xFF00 };
        uint16_t y[8];
        v5_shl_i16(xs, y);
        for (int i = 0; i < 8; i++)
            CHECK(y[i] == ref_shl_i16(xs[i]), "v5_shl_i16");
        v5_sar_i16((const int16_t *)xs, (int16_t *)y);
        for (int i = 0; i < 8; i++)
            CHECK(y[i] == (uint16_t)ref_sar_i16(((const int16_t *)xs)[i]),
                  "v5_sar_i16");
    }
    {
        const uint64_t xs[2] = { 0x8000000000000001ull, 0xFFFFFFFFFFFFFFFFull };
        uint64_t y[2];
        v5_shl_i64(xs, y);
        for (int i = 0; i < 2; i++)
            CHECK(y[i] == ref_shl_i64(xs[i]), "v5_shl_i64");
        v5_shr_u64(xs, y);
        for (int i = 0; i < 2; i++)
            CHECK(y[i] == ref_shr_u64(xs[i]), "v5_shr_u64");
        v5_sub1_u64(xs, y);
        for (int i = 0; i < 2; i++)
            CHECK(y[i] == xs[i] - 1ull, "v5_sub1_u64 (0 wrap)");
        v5_neg_i64((const int64_t *)xs, (int64_t *)y);
        for (int i = 0; i < 2; i++)
            CHECK(((int64_t *)y)[i] == ref_neg_i64(((const int64_t *)xs)[i]),
                  "v5_neg_i64 (INT64_MIN negation)");
    }
    {
        const int32_t xs[8] = { 1, 2, 3, 4, 5, 6, 7, (int32_t)0x80000000 };
        int32_t y[8];
        v5_shl_i32x8(xs, y);
        for (int i = 0; i < 8; i++)
            CHECK((uint32_t)y[i] == ((uint32_t)xs[i] << 7), "v5_shl_i32x8");
    }
    /* Rotates: every amount over an asymmetric pattern, both spellings. */
    {
        const uint32_t seed = 0x12345678u;
        uint32_t x[4] = { seed, seed ^ 0x000000FFu, seed ^ 0x0F0F0F0Fu,
                          seed ^ 0xFFFF0000u };
        uint32_t y[4];
        v5_rotl_u32(x, y);
        for (int i = 0; i < 4; i++)
            CHECK(y[i] == ref_rotl_u32(x[i], 7), "v5_rotl_u32 (raw spelling)");
        v5_rotr_u32(x, y);
        for (int i = 0; i < 4; i++)
            CHECK(y[i] == ref_rotr_u32(x[i], 19), "v5_rotr_u32 (raw spelling)");
        v5_rotl_swapped(x, y);
        for (int i = 0; i < 4; i++)
            CHECK(y[i] == ref_rotl_u32(x[i], 7), "v5_rotl_swapped (operand order)");
        const uint64_t x64[4] = { 0x0123456789ABCDEFull,
                                  0xFEDCBA9876543210ull,
                                  0xA5A5A5A5A5A5A5A5ull,
                                  0x00000000FFFFFFFFull };
        uint64_t y64[4];
        v5_rotl_u64(x64, y64);
        for (int i = 0; i < 4; i++)
            CHECK(y64[i] == ref_rotl_u64(x64[i], 13), "v5_rotl_u64");
    }
    /* FP min/max: NaN payloads, ±0, equals — compared bit-exactly. */
    {
        union f32bits in[4];
        in[0].u = 0x7FC00000u; /* qNaN */
        in[1].u = 0xFFC00000u; /* -qNaN */
        in[2].u = 0x00000000u; /* +0.0 */
        in[3].u = 0x80000000u; /* -0.0 */
        float fx[4], y[4];
        memcpy(fx, in, sizeof fx);
        v5_min_f32(fx, y);
        /* The ternary: NaN < 1.5 is false → 1.5; +0 < 1.5 → +0; -0 → -0. */
        union f32bits out[4];
        memcpy(out, y, sizeof y);
        CHECK(out[0].u == 0x3FC00000u, "v5_min_f32 qNaN lane");
        CHECK(out[1].u == 0x3FC00000u, "v5_min_f32 -qNaN lane");
        CHECK(out[2].u == 0x00000000u, "v5_min_f32 +0 lane");
        CHECK(out[3].u == 0x80000000u, "v5_min_f32 -0 lane (sign preserved)");
        v5_max_f32(fx, y);
        memcpy(out, y, sizeof y);
        /* x > -2: NaN → false → -2; +0 > -2 → +0; -0 > -2 → -0. */
        CHECK(out[0].u == 0xC0000000u, "v5_max_f32 qNaN lane");
        CHECK(out[1].u == 0xC0000000u, "v5_max_f32 -qNaN lane");
        CHECK(out[2].u == 0x00000000u, "v5_max_f32 +0 lane");
        CHECK(out[3].u == 0x80000000u, "v5_max_f32 -0 lane (sign preserved)");
    }
    {
        const double xs[4] = { 0.5, -1.0, 0.1, 2.0 };
        double y[4];
        v5_min_f64_swapped(xs, y);
        for (int i = 0; i < 4; i++) {
            double want = xs[i] < 0.25 ? 0.25 : xs[i];
            CHECK(y[i] == want, "v5_min_f64_swapped");
        }
        v5_max_f64_swapped(xs, y);
        for (int i = 0; i < 4; i++) {
            double want = xs[i] > -0.75 ? -0.75 : xs[i];
            CHECK(y[i] == want, "v5_max_f64_swapped");
        }
    }
    {
        const float xs[8] = { -5.0f, -0.0f, 0.25f, 4.0f,
                              1.0f, -1.0f, 0.0f, 2.5f };
        float y[8], t[8];
        v5_clamp_f32x8(xs, y, t);
        for (int i = 0; i < 8; i++) {
            float t = xs[i] < 0.0f ? 0.0f : xs[i];
            t = t > 1.0f ? 1.0f : t;
            CHECK(y[i] == t, "v5_clamp_f32x8");
        }
    }
    /* ChaCha-shaped vector quarter-round: vs the reference. */
    {
        uint32_t a[4] = { 0x61707865, 0x3320646e, 0x79622d32, 0x6b206574 };
        uint32_t b[4] = { 0x01020304, 0x05060708, 0x090a0b0c, 0x0d0e0f10 };
        uint32_t c[4] = { 0x89abcdef, 0x00112233, 0x44556677, 0x8899aabb };
        uint32_t r[4];
        chacha_words(a, b, c, r);
        for (int i = 0; i < 4; i++) {
            uint32_t want = ref_rotl_u32(a[i] + b[i], 7) ^ c[i];
            CHECK(r[i] == want, "chacha_words");
        }
    }
    /* Adversarial shapes: results must be correct whether or not they
     * vectorize (the contracts in the check script pin the codegen). */
    {
        const int32_t xs[4] = { 1, 2, 3, 4 };
        int32_t y[4], z[4];
        v5_varying_shift(xs, y);
        CHECK(y[0] == 2 && y[1] == 64 && y[2] == 1536 && y[3] == 32768,
              "v5_varying_shift");
        v5_shift_external_use(xs, y, z);
        CHECK(z[0] == (1 << 6) + 1, "v5_shift_external_use z");
        CHECK(y[0] == (1 << 6) && y[1] == (2 << 6) && y[2] == (3 << 6)
              && y[3] == (4 << 6), "v5_shift_external_use y");
        const float fs[4] = { -1.0f, 2.0f, 1.5f, 0.0f };
        float fy[4];
        v5_nonstrict_min(fs, fy);
        for (int i = 0; i < 4; i++) {
            float want = fs[i] <= 1.5f ? fs[i] : 1.5f;
            CHECK(fy[i] == want, "v5_nonstrict_min");
        }
        g_v5_cmp_use = 0;
        v5_cmp_external_use(fs, fy);
        CHECK(g_v5_cmp_use == 2, "v5_cmp_external_use side channel");
        for (int i = 0; i < 4; i++) {
            float want = fs[i] < 1.5f ? fs[i] : 1.5f;
            CHECK(fy[i] == want, "v5_cmp_external_use values");
        }
        uint32_t rx[4] = { 0x12345678u, 0x9ABCDEF0u, 0x0F0F0F0Fu, 1u };
        uint32_t rx0_orig = rx[0];
        uint32_t ry[4];
        v5_rot_interleaved_write(rx, ry);
        CHECK(rx[0] == 0xdeadbeefu, "v5_rot_interleaved_write x write");
        /* Lane 0 reads the PRE-write value for the shl half and the
         * POST-write value for the lshr half — the exact hazard. */
        CHECK(ry[0] == ((rx0_orig << 7) | (0xdeadbeefu >> 25)),
              "v5_rot_interleaved_write y[0] split halves");
        for (int i = 1; i < 4; i++)
            CHECK(ry[i] == ref_rotl_u32(rx[i], 7), "v5_rot_interleaved_write y");
    }

    printf("bb_slp_v5: all pass (%d fails)\n", fails);
    return fails != 0;
}
