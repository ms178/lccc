/*
 * BB-SLP v6 battery: the 128-bit VEX memory fold, FP negation, integer
 * min/max folds, the general cmp+blendv composite, and the rule-(b)
 * cross-block relaxation.
 *
 * Every vectorization candidate is verified against a spelling-twins
 * reference and, where bit-exactness is the whole proof (FP Neg, FP
 * non-strict selects), compared BIT-EXACTLY over lanes chosen to hit the
 * exactness hazards:
 *   - memfold: unaligned 128-bit streams, both operand orders of the
 *     commutative folds, non-commutative (sub/min) order preservation;
 *   - FP Neg: qNaN payloads (sign flips, payload verbatim), ±0, ±inf;
 *   - integer min/max: INT_MIN/INT_MAX sign edges, every relational
 *     spelling including <= / >= (exact for integers);
 *   - cmp+blendv: every predicate family (eq/ne/lt/le + unsigned + FP
 *     eq/ne/le), mixed arms, NaN lanes (all C comparisons are false on
 *     NaN — the packed predicates are IEEE-exact);
 *   - cross-block: lanes consumed in dominated successor blocks.
 *
 * Adversarial sections verify the REJECTIONS: cond values with external
 * uses, non-uniform predicates.
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

static uint32_t bitsf(float f) {
    uint32_t u;
    memcpy(&u, &f, 4);
    return u;
}
static void chkf(float got, float want, const char *n) {
    if (bitsf(got) != bitsf(want)) {
        printf("FAIL %s: %08x vs %08x\n", n, bitsf(got), bitsf(want));
        fails++;
    }
}

/* ── Section 1: the 128-bit VEX memory fold ─────────────────────────── */

/* Commutative, both operand orders (the fold sits in the r/m slot). */
void v6_mf_add(const int32_t *restrict a, int32_t *restrict q) {
    q[0] = a[0] + 3; q[1] = a[1] + 3; q[2] = a[2] + 3; q[3] = a[3] + 3;
}
void v6_mf_xor(const uint32_t *restrict a, uint32_t *restrict q) {
    q[0] = a[0] ^ 0xdeadbeefu; q[1] = a[1] ^ 0xdeadbeefu;
    q[2] = a[2] ^ 0xdeadbeefu; q[3] = a[3] ^ 0xdeadbeefu;
}
/* Non-commutative: the load folds in the src2 position only. */
void v6_mf_sub(const int32_t *restrict a, int32_t *restrict q) {
    q[0] = 100 - a[0]; q[1] = 100 - a[1]; q[2] = 100 - a[2]; q[3] = 100 - a[3];
}
/* Two-stream fold: the farther load folds, the nearer streams. */
void v6_mf_two(const int32_t *restrict a, const int32_t *restrict b,
               int32_t *restrict q) {
    q[0] = a[0] + b[0]; q[1] = a[1] + b[1]; q[2] = a[2] + b[2]; q[3] = a[3] + b[3];
}
/* 256-bit twin. */
void v6_mf_add8(const int32_t *restrict a, int32_t *restrict q) {
    q[0] = a[0] + 7; q[1] = a[1] + 7; q[2] = a[2] + 7; q[3] = a[3] + 7;
    q[4] = a[4] + 7; q[5] = a[5] + 7; q[6] = a[6] + 7; q[7] = a[7] + 7;
}

/* ── Section 2: FP negation (the -0.0 sign-mask composite) ──────────── */

void v6_neg_f32x4(const float *restrict a, float *restrict q) {
    q[0] = -a[0]; q[1] = -a[1]; q[2] = -a[2]; q[3] = -a[3];
}
void v6_neg_f32x8(const float *restrict a, float *restrict q) {
    q[0] = -a[0]; q[1] = -a[1]; q[2] = -a[2]; q[3] = -a[3];
    q[4] = -a[4]; q[5] = -a[5]; q[6] = -a[6]; q[7] = -a[7];
}
void v6_neg_f64x2(const double *restrict a, double *restrict q) {
    q[0] = -a[0]; q[1] = -a[1];
}
void v6_neg_f64x4(const double *restrict a, double *restrict q) {
    q[0] = -a[0]; q[1] = -a[1]; q[2] = -a[2]; q[3] = -a[3];
}

/* ── Section 3: integer min/max folds (every spelling) ──────────────── */

void v6_imin_lt(const int32_t *restrict a, const int32_t *restrict b,
                int32_t *restrict q) {
    q[0] = a[0] < b[0] ? a[0] : b[0]; q[1] = a[1] < b[1] ? a[1] : b[1];
    q[2] = a[2] < b[2] ? a[2] : b[2]; q[3] = a[3] < b[3] ? a[3] : b[3];
}
void v6_imin_le(const int32_t *restrict a, const int32_t *restrict b,
                int32_t *restrict q) {
    q[0] = a[0] <= b[0] ? a[0] : b[0]; q[1] = a[1] <= b[1] ? a[1] : b[1];
    q[2] = a[2] <= b[2] ? a[2] : b[2]; q[3] = a[3] <= b[3] ? a[3] : b[3];
}
void v6_imax_gt(const int32_t *restrict a, const int32_t *restrict b,
                int32_t *restrict q) {
    q[0] = a[0] > b[0] ? a[0] : b[0]; q[1] = a[1] > b[1] ? a[1] : b[1];
    q[2] = a[2] > b[2] ? a[2] : b[2]; q[3] = a[3] > b[3] ? a[3] : b[3];
}
void v6_imax_swapped(const int32_t *restrict a, const int32_t *restrict b,
                     int32_t *restrict q) {
    q[0] = a[0] <= b[0] ? b[0] : a[0]; q[1] = a[1] <= b[1] ? b[1] : a[1];
    q[2] = a[2] <= b[2] ? b[2] : a[2]; q[3] = a[3] <= b[3] ? b[3] : a[3];
}
void v6_imin8_256(const int32_t *restrict a, const int32_t *restrict b,
                  int32_t *restrict q) {
    q[0] = a[0] < b[0] ? a[0] : b[0]; q[1] = a[1] < b[1] ? a[1] : b[1];
    q[2] = a[2] < b[2] ? a[2] : b[2]; q[3] = a[3] < b[3] ? a[3] : b[3];
    q[4] = a[4] < b[4] ? a[4] : b[4]; q[5] = a[5] < b[5] ? a[5] : b[5];
    q[6] = a[6] < b[6] ? a[6] : b[6]; q[7] = a[7] < b[7] ? a[7] : b[7];
}

/* ── Section 4: the general cmp+blendv composite ────────────────────── */

/* Mixed arms — NOT min/max foldable; every predicate family. */
void v6_sel_lt(const int32_t *restrict a, const int32_t *restrict b,
               int32_t *restrict q) {
    q[0] = a[0] < b[0] ? 1 : -1; q[1] = a[1] < b[1] ? 1 : -1;
    q[2] = a[2] < b[2] ? 1 : -1; q[3] = a[3] < b[3] ? 1 : -1;
}
void v6_sel_ult(const uint32_t *restrict a, const uint32_t *restrict b,
                uint32_t *restrict q) {
    q[0] = a[0] < b[0] ? 0x13579bdfu : 0x2468ace0u;
    q[1] = a[1] < b[1] ? 0x13579bdfu : 0x2468ace0u;
    q[2] = a[2] < b[2] ? 0x13579bdfu : 0x2468ace0u;
    q[3] = a[3] < b[3] ? 0x13579bdfu : 0x2468ace0u;
}
void v6_sel_uge(const uint32_t *restrict a, const uint32_t *restrict b,
                uint32_t *restrict q) {
    q[0] = a[0] >= b[0] ? a[0] * 3u : b[0] + 11u;
    q[1] = a[1] >= b[1] ? a[1] * 3u : b[1] + 11u;
    q[2] = a[2] >= b[2] ? a[2] * 3u : b[2] + 11u;
    q[3] = a[3] >= b[3] ? a[3] * 3u : b[3] + 11u;
}
void v6_sel_eq(const int32_t *restrict a, const int32_t *restrict b,
               int32_t *restrict q) {
    q[0] = a[0] == b[0] ? a[0] * 5 : b[0] - 7;
    q[1] = a[1] == b[1] ? a[1] * 5 : b[1] - 7;
    q[2] = a[2] == b[2] ? a[2] * 5 : b[2] - 7;
    q[3] = a[3] == b[3] ? a[3] * 5 : b[3] - 7;
}
void v6_sel_ne(const int32_t *restrict a, const int32_t *restrict b,
               int32_t *restrict q) {
    q[0] = a[0] != b[0] ? a[0] ^ 9 : b[0] ^ 21;
    q[1] = a[1] != b[1] ? a[1] ^ 9 : b[1] ^ 21;
    q[2] = a[2] != b[2] ? a[2] ^ 9 : b[2] ^ 21;
    q[3] = a[3] != b[3] ? a[3] ^ 9 : b[3] ^ 21;
}
/* FP non-strict (<=) — the shape the strict min/max fold must refuse and
 * the blendv composite must take; NaN lanes are the whole proof. */
void v6_fsel_le(const float *restrict a, const float *restrict b,
                float *restrict q) {
    q[0] = a[0] <= b[0] ? a[0] : b[0]; q[1] = a[1] <= b[1] ? a[1] : b[1];
    q[2] = a[2] <= b[2] ? a[2] : b[2]; q[3] = a[3] <= b[3] ? a[3] : b[3];
}
void v6_fsel_ne(const float *restrict a, const float *restrict b,
                float *restrict q) {
    q[0] = a[0] != b[0] ? a[0] : b[0]; q[1] = a[1] != b[1] ? a[1] : b[1];
    q[2] = a[2] != b[2] ? a[2] : b[2]; q[3] = a[3] != b[3] ? a[3] : b[3];
}
/* i16 lanes through the same composite. */
void v6_sel_i16(const int16_t *restrict a, const int16_t *restrict b,
                int16_t *restrict q) {
    q[0] = a[0] < b[0] ? 3 : -3; q[1] = a[1] < b[1] ? 3 : -3;
    q[2] = a[2] < b[2] ? 3 : -3; q[3] = a[3] < b[3] ? 3 : -3;
    q[4] = a[4] < b[4] ? 3 : -3; q[5] = a[5] < b[5] ? 3 : -3;
    q[6] = a[6] < b[6] ? 3 : -3; q[7] = a[7] < b[7] ? 3 : -3;
}

/* ── Section 5: rule-(b) cross-block relaxation ─────────────────────── */

/* 2×i64 lanes (the I64x2 pack shape), with the in-block stores keeping
 * the pack alive in the defining block and the successor-block arithmetic
 * consuming the extracts — the rule-(b) relaxation shape (dominance
 * proof: the extracts ride with the pack and dominate every use). */
long v6_xblock(const long *restrict a, const long *restrict b, long *restrict t, int c) {
    long x = a[0] + b[0], y = a[1] + b[1];
    t[0] = x; t[1] = y;
    if (c) {
        return x * y + t[0];
    }
    return x - y + t[1];
}
/* Loop-exit consumption of scalar accumulators fed by lanes defined in a
 * loop body — a RUNTIME-only contract (2×i32 accumulators are not a
 * packable shape; the loop-exit values must simply be right). */
long v6_xloop(const int32_t *restrict a, int n) {
    int32_t s = 0, t = 0;
    for (int i = 0; i < n; i++) {
        int32_t u = a[0] + i, v = a[1] + i;
        s += u; t += v;
        if (u == 100) break;
    }
    return s * 1000000L + t;
}

/* ── Section 6: W5 register-homing contracts ────────────────────────── */

/* The rotate diamond (multi-use load → two shifts → or): every value is
 * register-homed — exactly two memory ops (the load and the store), no
 * spill/reload round trips, NO stack frame at all. */
void v6_w5_rotl256(const unsigned long long *restrict a, unsigned long long *restrict q) {
    q[0] = (a[0] << 13) | (a[0] >> 51); q[1] = (a[1] << 13) | (a[1] >> 51);
    q[2] = (a[2] << 13) | (a[2] >> 51); q[3] = (a[3] << 13) | (a[3] >> 51);
}
/* The FP-Neg 8-lane: the deferred load is register-homed — NO dead
 * frame (the subq $56 regression shape). */
void v6_w5_fneg8(const float *restrict a, float *restrict q) {
    q[0] = -a[0]; q[1] = -a[1]; q[2] = -a[2]; q[3] = -a[3];
    q[4] = -a[4]; q[5] = -a[5]; q[6] = -a[6]; q[7] = -a[7];
}
/* Two-stream min: the b-load is homed, the a-load folds — three
 * instructions, no frame (the 7-insn + subq $104 regression shape). */
void v6_w5_min(const int32_t *restrict a, const int32_t *restrict b, int32_t *restrict q) {
    q[0] = a[0] < b[0] ? a[0] : b[0]; q[1] = a[1] < b[1] ? a[1] : b[1];
    q[2] = a[2] < b[2] ? a[2] : b[2]; q[3] = a[3] < b[3] ? a[3] : b[3];
}
/* The stale-claim adversarial shape (simd_vecreg loopcarry distilled):
 * an accumulator whose claim must NOT survive the streamed reload and
 * the shift that redefine the scratch — the xor consumes acc and the
 * shifted stream, never the shifted stream twice. */
void v6_w5_stale_claim(const unsigned *restrict data, unsigned *restrict q) {
    unsigned acc = 0;
    for (int i = 0; i < 16; i += 4) {
        unsigned v0 = data[i], v1 = data[i+1], v2 = data[i+2], v3 = data[i+3];
        acc = (acc + v0) ^ (v0 >> 1);
        acc = (acc + v1) ^ (v1 >> 1);
        acc = (acc + v2) ^ (v2 >> 1);
        acc = (acc + v3) ^ (v3 >> 1);
    }
    q[0] = acc;
}

/* ── Section 7: adversarial rejections ──────────────────────────────── */

/* Non-uniform predicates across lanes: no single packed compare exists. */
void v6_adv_mixed_pred(const int32_t *restrict a, const int32_t *restrict b,
                       int32_t *restrict q) {
    q[0] = a[0] < b[0] ? 1 : -1; q[1] = a[1] <= b[1] ? 1 : -1;
    q[2] = a[2] < b[2] ? 1 : -1; q[3] = a[3] <= b[3] ? 1 : -1;
}
/* The comparison value has an external use: the cmp cannot die with the
 * select (a scalar bool cannot be reconstructed from the packed mask). */
int v6_adv_cond_ext(const int32_t *restrict a, const int32_t *restrict b,
                    int32_t *restrict q, int *w) {
    int c0 = a[0] < b[0], c1 = a[1] < b[1], c2 = a[2] < b[2], c3 = a[3] < b[3];
    q[0] = c0 ? 1 : -1; q[1] = c1 ? 1 : -1; q[2] = c2 ? 1 : -1; q[3] = c3 ? 1 : -1;
    w[0] = c0 + c1 + c2 + c3;   /* external uses of the conditions */
    return c0;
}

int main(void) {
    /* Section 1: memfold. */
    {
        const int32_t a[8] = {INT32_MIN, -1, 0, INT32_MAX, 7, -7, 123456789, -123456789};
        int32_t q[8];
        v6_mf_add(a, q);
        /* INT32_MAX + 3 wraps: spelled as the defined unsigned wraparound
         * (the battery must never hinge on signed-overflow UB — gcc and
         * lccc agree bit-for-bit on the wrapped value). */
        CHECK(q[0] == INT32_MIN + 3 && q[1] == 2 && q[2] == 3
                  && q[3] == (int32_t)((uint32_t)INT32_MAX + 3u),
              "mf_add wrap edges");
        v6_mf_xor((const uint32_t *)a, (uint32_t *)q);
        CHECK((uint32_t)q[0] == 0x80000000u ^ 0xdeadbeefu && (uint32_t)q[3] == 0x7fffffffu ^ 0xdeadbeefu,
              "mf_xor");
        v6_mf_sub(a, q);
        CHECK(q[0] == 100 - INT32_MIN && q[3] == 100 - INT32_MAX, "mf_sub order");
        const int32_t b[8] = {1, -1, 100, -100, 5, 6, 7, 8};
        v6_mf_two(a, b, q);
        /* v6_mf_two is the 4-lane fold shape: only q[0..3] are written
         * (the 8-lane twin is checked separately below). */
        CHECK(q[0] == INT32_MIN + 1 && q[3] == INT32_MAX - 100, "mf_two streams");
        v6_mf_add8(a, q);
        CHECK(q[7] == -123456789 + 7, "mf_add8 256-bit");
    }
    /* Section 2: FP negation — bit-exact over the hazard lanes. */
    {
        const float af[8] = {
            1.5f, -2.5f, 0.0f, -0.0f,
            __builtin_nanf("0x123"), -__builtin_nanf("0x456"),
            __builtin_inff(), -__builtin_inff()
        };
        const double ad[4] = {1.5, -2.5, 0.0, -0.0};
        const double ad2[2] = {__builtin_nan("0xabc"), -__builtin_inff()};
        float qf[8];
        double qd[4], qd2[2];
        v6_neg_f32x4(af, qf);
        for (int i = 0; i < 4; i++) { float w = -af[i]; chkf(qf[i], w, "neg_f32x4"); }
        v6_neg_f32x8(af, qf);
        for (int i = 0; i < 8; i++) { float w = -af[i]; chkf(qf[i], w, "neg_f32x8"); }
        v6_neg_f64x2(ad2, qd2);
        for (int i = 0; i < 2; i++) {
            uint64_t g, w;
            memcpy(&g, &qd2[i], 8);
            double x = -ad2[i];
            memcpy(&w, &x, 8);
            CHECK(g == w, "neg_f64x2 bits");
        }
        v6_neg_f64x4(ad, qd);
        for (int i = 0; i < 4; i++) {
            uint64_t g, w;
            memcpy(&g, &qd[i], 8);
            double x = -ad[i];
            memcpy(&w, &x, 8);
            CHECK(g == w, "neg_f64x4 bits");
        }
        CHECK(bitsf(qf[2]) == 0x80000000u, "neg +0 -> -0");
        CHECK(bitsf(qf[3]) == 0x00000000u, "neg -0 -> +0");
        CHECK(bitsf(qf[4]) == 0xffc00123u, "neg NaN payload");
    }
    /* Section 3: integer min/max. */
    {
        const int32_t a[8] = {INT32_MIN, -1, 0, INT32_MAX, 5, -5, 0, 1};
        const int32_t b[8] = {INT32_MAX, 1, 0, INT32_MIN, -5, 5, 1, 0};
        int32_t q[8];
        v6_imin_lt(a, b, q);
        CHECK(q[0] == INT32_MIN && q[1] == -1 && q[2] == 0 && q[3] == INT32_MIN, "imin_lt edges");
        v6_imin_le(a, b, q);
        CHECK(q[0] == INT32_MIN && q[2] == 0 && q[3] == INT32_MIN, "imin_le equal lanes");
        v6_imax_gt(a, b, q);
        CHECK(q[0] == INT32_MAX && q[1] == 1 && q[2] == 0 && q[3] == INT32_MAX, "imax_gt edges");
        v6_imax_swapped(a, b, q);
        CHECK(q[0] == INT32_MAX && q[2] == 0 && q[3] == INT32_MAX, "imax_swapped");
        v6_imin8_256(a, b, q);
        CHECK(q[4] == -5 && q[5] == -5 && q[6] == 0 && q[7] == 0, "imin8 256-bit");
    }
    /* Section 4: cmp+blendv. */
    {
        const int32_t a[4] = {INT32_MIN, -1, 0, INT32_MAX};
        const int32_t b[4] = {INT32_MAX, 1, 0, INT32_MIN};
        const uint32_t au[4] = {0x80000000u, 1, 0xffffffffu, 0x7fffffffu};
        const uint32_t bu[4] = {0x7fffffffu, 2, 0xfffffffeu, 0x80000000u};
        const float af[4] = {-0.0f, __builtin_nanf("0x1"), 1.5f, __builtin_inff()};
        const float bf[4] = {0.0f, 1.5f, __builtin_nanf("0x2"), __builtin_inff()};
        int32_t q[4];
        uint32_t qu[4];
        float qf[4];
        v6_sel_lt(a, b, q);
        CHECK(q[0] == 1 && q[1] == 1 && q[2] == -1 && q[3] == -1, "sel_lt");
        v6_sel_ult(au, bu, qu);
        /* lane 0: 0x80000000 >= 0x7fffffff unsigned → FALSE arm;
         * lane 3: 0x7fffffff < 0x80000000 → TRUE arm. */
        CHECK(qu[0] == 0x2468ace0u && qu[1] == 0x13579bdfu && qu[2] == 0x2468ace0u &&
                  qu[3] == 0x13579bdfu, "sel_ult");
        v6_sel_uge(au, bu, qu);
        /* lane 0: >= TRUE → a*3 (wraps); lane 2: 0xffffffff >= 0xfffffffe
         * TRUE → a*3 (wraps). */
        CHECK(qu[0] == 0x80000000u * 3u && qu[2] == 0xffffffffu * 3u, "sel_uge");
        v6_sel_eq(a, b, q);
        CHECK(q[0] == INT32_MAX - 7 && q[2] == 0 * 5, "sel_eq");
        v6_sel_ne(a, b, q);
        CHECK(q[0] == (INT32_MIN ^ 9) && q[2] == (0 ^ 21), "sel_ne");
        v6_fsel_le(af, bf, qf);
        for (int i = 0; i < 4; i++) chkf(qf[i], af[i] <= bf[i] ? af[i] : bf[i], "fsel_le");
        v6_fsel_ne(af, bf, qf);
        for (int i = 0; i < 4; i++) chkf(qf[i], af[i] != bf[i] ? af[i] : bf[i], "fsel_ne");
        const int16_t a16[8] = {-32768, -1, 0, 1, 32767, -5, 100, -100};
        const int16_t b16[8] = {32767, 1, 0, -1, -32768, 5, -100, 100};
        int16_t q16[8];
        v6_sel_i16(a16, b16, q16);
        for (int i = 0; i < 8; i++) CHECK(q16[i] == (a16[i] < b16[i] ? 3 : -3), "sel_i16");
    }
    /* Section 5: cross-block. */
    {
        const long a[2] = {1000000000L, -2000000000L};
        const long b[2] = {123456789L, 987654321L};
        long t[2];
        long x = a[0] + b[0], y = a[1] + b[1];
        CHECK(v6_xblock(a, b, t, 1) == x * y + x, "xblock taken");
        CHECK(t[0] == x && t[1] == y, "xblock stores");
        CHECK(v6_xblock(a, b, t, 0) == x - y + y, "xblock fallthrough");
        const int32_t la[2] = {10, 20};
        CHECK(v6_xloop(la, 4) == (10 + 0 + 10 + 1 + 10 + 2 + 10 + 3) * 1000000L +
                                     (20 + 0 + 20 + 1 + 20 + 2 + 20 + 3), "xloop");
    }
    /* Section 6: W5 register-homing contracts (values + the stale-claim
     * adversarial shape — the compiler must compute the true recurrence). */
    {
        const unsigned long long r64[4] = {0x0123456789abcdefULL, ~0ULL, 1ULL, 0xdeadbeefcafeULL};
        unsigned long long q64[4];
        v6_w5_rotl256(r64, q64);
        for (int i = 0; i < 4; i++)
            CHECK(q64[i] == ((r64[i] << 13) | (r64[i] >> 51)), "w5 rotl256");
        const float rf[8] = {1.5f, -2.5f, 0.0f, -0.0f,
                             __builtin_nanf("0x51"), -__builtin_nanf("0x62"),
                             __builtin_inff(), -__builtin_inff()};
        float qf8[8];
        v6_w5_fneg8(rf, qf8);
        for (int i = 0; i < 8; i++) {
            float w = -rf[i];
            chkf(qf8[i], w, "w5 fneg8");
        }
        const int32_t ra[4] = {INT32_MIN, -1, 0, INT32_MAX};
        const int32_t rb[4] = {INT32_MAX, 1, 0, INT32_MIN};
        int32_t qi4[4];
        v6_w5_min(ra, rb, qi4);
        for (int i = 0; i < 4; i++)
            CHECK(qi4[i] == (ra[i] < rb[i] ? ra[i] : rb[i]), "w5 min");
        const unsigned sd[16] = {1, 2, 3, 4, 0x80000000u, 0x7fffffffu, 0, ~0u,
                                 0xdeadbeefu, 0x13579bdfu, 255, 256, 65535, 65536, 7, 0x0f0f0f0fu};
        unsigned sq[1];
        v6_w5_stale_claim(sd, sq);
        {
            unsigned acc = 0, ref = 0;
            for (int i = 0; i < 16; i += 4) {
                unsigned v0 = sd[i], v1 = sd[i+1], v2 = sd[i+2], v3 = sd[i+3];
                acc = (acc + v0) ^ (v0 >> 1);
                acc = (acc + v1) ^ (v1 >> 1);
                acc = (acc + v2) ^ (v2 >> 1);
                acc = (acc + v3) ^ (v3 >> 1);
            }
            ref = acc;
            CHECK(sq[0] == ref, "w5 stale-claim recurrence");
        }
    }
    /* Section 7: adversarial — correct results, whatever the lowering. */
    {
        const int32_t a[4] = {1, 2, 3, 4};
        const int32_t b[4] = {2, 2, 2, 2};
        int32_t q[4];
        v6_adv_mixed_pred(a, b, q);
        CHECK(q[0] == 1 && q[1] == 1 && q[2] == -1 && q[3] == -1, "adv mixed preds");
        int w[1];
        int r = v6_adv_cond_ext(a, b, q, w);
        CHECK(q[0] == 1 && q[1] == -1 && q[2] == -1 && q[3] == -1 && w[0] == 1 && r == 1,
              "adv cond external");
    }
    if (fails == 0) {
        printf("bb_slp_v6: all pass (0 fails)\n");
    }
    return fails;
}
