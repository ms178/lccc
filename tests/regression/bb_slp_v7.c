/*
 * BB-SLP v7 red-team battery: adversarial edges of the v6 feature set
 * that the v6 battery does not reach. Every vectorization candidate is
 * verified against a spelling-twins reference over lanes chosen to hit
 * the exactness hazards; the whole battery is run tri-config (SLP on /
 * CCC_NO_BB_SLP=1 / gcc -O2 -march=x86-64-v3) by the gate script and
 * must be bit-identical in all three.
 *
 *   - cross-block rule-(b) edges: self-loop backedge phis (rejection),
 *     phi incomings in dominated successors, nested domination, mixed
 *     in-block + cross-block uses of the same lanes, stored-and-live-out;
 *   - cmp+blendv corners: i8 lanes (vpblendvb), f64 lanes, mirrored FP
 *     relations on NaN lanes, unsigned relations on sign-bit patterns,
 *     a compare consumed by two selects (rejection), a > b spelled both
 *     ways;
 *   - int min/max corners: U32 lanes under SIGNED compares (pminsd on
 *     raw bits), I16 256-bit, U8 128-bit, nested ternary chains;
 *   - FP-Neg chains: -(-x), neg feeding arithmetic, neg stored and
 *     live-out cross-block;
 *   - memfold corners: f32x4/f64x2/i16x8 streams, both operand orders,
 *     shift consumers in the same block (must materialize, never the
 *     removed register-only VEX shift fold), unhomed destinations;
 *   - scheduling: splat-early under register pressure, many-parameter
 *     prologues (ParamRef prefix), live ranges crossing whole blocks.
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
static uint64_t bitsd(double d) {
    uint64_t u;
    memcpy(&u, &d, 8);
    return u;
}
static void chkf(float got, float want, const char *n) {
    if (bitsf(got) != bitsf(want)) {
        printf("FAIL %s: %08x vs %08x\n", n, bitsf(got), bitsf(want));
        fails++;
    }
}
static void chkd(double got, double want, const char *n) {
    if (bitsd(got) != bitsd(want)) {
        printf("FAIL %s: %016llx vs %016llx\n", n,
               (unsigned long long)bitsd(got), (unsigned long long)bitsd(want));
        fails++;
    }
}

/* ── Section A: cross-block rule-(b) adversarial edges ──────────────── */

/* A1: self-loop backedge phi. The lanes x0..x3 are defined in the loop
 * body and consumed by the loop-header phi on the backedge — an in-block
 * use BEFORE the pack slot. The pack must be rejected (or scheduled
 * legally); the values must be exact either way. */
uint32_t v7_x_selfloop(const uint32_t *restrict a, int n) {
    uint32_t x0 = 1, x1 = 2, x2 = 3, x3 = 4;
    uint32_t acc = 0;
    for (int i = 0; i < n; i++) {
        acc += x0;
        x0 = a[0] + i; x1 = a[1] + i; x2 = a[2] + i; x3 = a[3] + i;
    }
    return acc + x0 + x1 + x2 + x3;
}

/* A2: phi incoming in a dominated successor — the extract must feed the
 * phi of the merge block. */
uint32_t v7_x_phi(const uint32_t *restrict a, uint32_t *restrict t, int c) {
    uint32_t x0 = a[0] + 7, x1 = a[1] + 7, x2 = a[2] + 7, x3 = a[3] + 7;
    t[0] = x0; t[1] = x1; t[2] = x2; t[3] = x3;
    uint32_t p;
    if (c) {
        p = x0 ^ x1;
    } else {
        p = x2 | x3;
    }
    return p + (x0 & x1) + (x2 ^ x3);
}

/* A3: nested domination — uses two levels below the defining block. */
uint32_t v7_x_nested(const uint32_t *restrict a, uint32_t *restrict t, int c, int d) {
    uint32_t x0 = a[0] * 3, x1 = a[1] * 3, x2 = a[2] * 3, x3 = a[3] * 3;
    t[0] = x0; t[1] = x1; t[2] = x2; t[3] = x3;
    uint32_t r = 0;
    if (c) {
        if (d) {
            r = x0 + x1 * 2;
        } else {
            r = x2 + x3 * 4;
        }
    }
    return r;
}

/* A4: mixed in-block and cross-block uses of the same lanes. */
uint32_t v7_x_mixed(const uint32_t *restrict a, uint32_t *restrict t, int c) {
    uint32_t x0 = a[0] - 5, x1 = a[1] - 5, x2 = a[2] - 5, x3 = a[3] - 5;
    uint32_t inblock = (x0 + x1) ^ (x2 + x3);   /* in-block, after pack */
    t[0] = x0; t[1] = x1; t[2] = x2; t[3] = x3; /* seed stores */
    if (c) {
        return inblock + x0 * x3;
    }
    return inblock + x1 + x2;
}

/* A5: stored in-block AND live-out (the store seeds the pack; the
 * successor reads the same SSA values, not the memory). */
uint32_t v7_x_stored_liveout(const uint64_t *restrict a, uint64_t *restrict t, int c) {
    uint64_t x = a[0] + a[1], y = a[2] + a[3];
    t[0] = x; t[1] = y;
    if (c) {
        return (uint32_t)(x * 3 + y);
    }
    return (uint32_t)(x ^ y);
}

/* ── Section B: cmp+blendv corners ──────────────────────────────────── */

/* B1: i8 lanes — the vpblendvb family. */
void v7_sel_i8(const int8_t *restrict a, const int8_t *restrict b,
               int8_t *restrict q) {
    q[0] = a[0] < b[0] ? 7 : -7; q[1] = a[1] < b[1] ? 7 : -7;
    q[2] = a[2] < b[2] ? 7 : -7; q[3] = a[3] < b[3] ? 7 : -7;
    q[4] = a[4] < b[4] ? 7 : -7; q[5] = a[5] < b[5] ? 7 : -7;
    q[6] = a[6] < b[6] ? 7 : -7; q[7] = a[7] < b[7] ? 7 : -7;
    q[8] = a[8] < b[8] ? 7 : -7; q[9] = a[9] < b[9] ? 7 : -7;
    q[10] = a[10] < b[10] ? 7 : -7; q[11] = a[11] < b[11] ? 7 : -7;
    q[12] = a[12] < b[12] ? 7 : -7; q[13] = a[13] < b[13] ? 7 : -7;
    q[14] = a[14] < b[14] ? 7 : -7; q[15] = a[15] < b[15] ? 7 : -7;
}

/* B2: f64 lanes, non-strict predicate (<=) with NaN lanes. */
void v7_fsel_f64(const double *restrict a, const double *restrict b,
                 double *restrict q) {
    q[0] = a[0] <= b[0] ? a[0] : b[0];
    q[1] = a[1] <= b[1] ? a[1] : b[1];
}

/* B3: mirrored FP relation (a >= b spelled directly) with NaN lanes.
 * Arms carry no NaN arithmetic: the false arm copies b verbatim, so the
 * bit-exact check is safe on NaN lanes. */
void v7_fsel_ge(const float *restrict a, const float *restrict b,
                float *restrict q) {
    q[0] = a[0] >= b[0] ? a[0] * 2.0f : b[0];
    q[1] = a[1] >= b[1] ? a[1] * 2.0f : b[1];
    q[2] = a[2] >= b[2] ? a[2] * 2.0f : b[2];
    q[3] = a[3] >= b[3] ? a[3] * 2.0f : b[3];
}

/* B4: unsigned relation on sign-bit patterns — the compare must use the
 * unsigned predicate, never the signed one, on identical bit patterns. */
void v7_sel_ugt(const uint32_t *restrict a, const uint32_t *restrict b,
                uint32_t *restrict q) {
    q[0] = a[0] > b[0] ? 0x11111111u : 0xeeeeeeeeu;
    q[1] = a[1] > b[1] ? 0x11111111u : 0xeeeeeeeeu;
    q[2] = a[2] > b[2] ? 0x11111111u : 0xeeeeeeeeu;
    q[3] = a[3] > b[3] ? 0x11111111u : 0xeeeeeeeeu;
}
/* The signed twin on the same bits — both must be right. */
void v7_sel_sgt(const uint32_t *restrict a, const uint32_t *restrict b,
                uint32_t *restrict q) {
    q[0] = ((int32_t)a[0] > (int32_t)b[0]) ? 0x11111111u : 0xeeeeeeeeu;
    q[1] = ((int32_t)a[1] > (int32_t)b[1]) ? 0x11111111u : 0xeeeeeeeeu;
    q[2] = ((int32_t)a[2] > (int32_t)b[2]) ? 0x11111111u : 0xeeeeeeeeu;
    q[3] = ((int32_t)a[3] > (int32_t)b[3]) ? 0x11111111u : 0xeeeeeeeeu;
}

/* B5: one compare consumed by two selects — the cond has external uses;
 * rejection is mandatory, values must be exact. */
void v7_sel_shared_cmp(const int32_t *restrict a, const int32_t *restrict b,
                       int32_t *restrict q, int32_t *restrict r) {
    for (int i = 0; i < 4; i++) {
        int c = a[i] < b[i];
        q[i] = c ? 1 : 2;
        r[i] = c ? 10 : 20;
    }
}

/* B6: a > b with the operands and arms both mirrored — the arms feed the
 * blend in the opposite order; NaN lanes must select the FALSE arm. */
void v7_fsel_mirror(const float *restrict a, const float *restrict b,
                    float *restrict q) {
    q[0] = a[0] > b[0] ? b[0] : a[0];
    q[1] = a[1] > b[1] ? b[1] : a[1];
    q[2] = a[2] > b[2] ? b[2] : a[2];
    q[3] = a[3] > b[3] ? b[3] : a[3];
}

/* ── Section C: int min/max corners ─────────────────────────────────── */

/* C1: U32 lanes under a SIGNED compare — pminsd/pmaxsd on the raw bits
 * is exact for the signed spelling; the unsigned arrays only carry the
 * lane VALUES. Edge lanes: 0x80000000 (INT_MIN as signed, huge as
 * unsigned). */
void v7_umin_signed_cmp(const uint32_t *restrict a, const uint32_t *restrict b,
                        uint32_t *restrict q) {
    q[0] = ((int32_t)a[0] < (int32_t)b[0]) ? a[0] : b[0];
    q[1] = ((int32_t)a[1] < (int32_t)b[1]) ? a[1] : b[1];
    q[2] = ((int32_t)a[2] < (int32_t)b[2]) ? a[2] : b[2];
    q[3] = ((int32_t)a[3] < (int32_t)b[3]) ? a[3] : b[3];
}

/* C2: I16 256-bit min (8 lanes). */
void v7_imin_i16x8(const int16_t *restrict a, const int16_t *restrict b,
                   int16_t *restrict q) {
    q[0] = a[0] < b[0] ? a[0] : b[0]; q[1] = a[1] < b[1] ? a[1] : b[1];
    q[2] = a[2] < b[2] ? a[2] : b[2]; q[3] = a[3] < b[3] ? a[3] : b[3];
    q[4] = a[4] < b[4] ? a[4] : b[4]; q[5] = a[5] < b[5] ? a[5] : b[5];
    q[6] = a[6] < b[6] ? a[6] : b[6]; q[7] = a[7] < b[7] ? a[7] : b[7];
}

/* C3: U8 128-bit min (pminub, 16 lanes). */
void v7_umin_u8x16(const uint8_t *restrict a, const uint8_t *restrict b,
                   uint8_t *restrict q) {
    for (int i = 0; i < 16; i++) {
        q[i] = a[i] < b[i] ? a[i] : b[i];
    }
}

/* C4: nested ternary chain — max(min(a,b), c) style, two packs deep. */
void v7_clamp(const int32_t *restrict a, const int32_t *restrict b,
              const int32_t *restrict c, int32_t *restrict q) {
    for (int i = 0; i < 4; i++) {
        int32_t lo = a[i] < b[i] ? a[i] : b[i];
        q[i] = lo > c[i] ? lo : c[i];
    }
}

/* ── Section D: FP-Neg chains ───────────────────────────────────────── */

/* D1: neg-of-neg — two chained FpNeg packs (sign flips cancel). */
void v7_negneg_f32(const float *restrict a, float *restrict q) {
    q[0] = -(-a[0]); q[1] = -(-a[1]); q[2] = -(-a[2]); q[3] = -(-a[3]);
}

/* D2: neg feeding arithmetic — the neg pack's result consumed by an add
 * pack (both widths exercised). */
void v7_neg_arith(const float *restrict a, const float *restrict b,
                  float *restrict q) {
    q[0] = -a[0] + b[0]; q[1] = -a[1] + b[1]; q[2] = -a[2] + b[2];
    q[3] = -a[3] + b[3];
}

/* D3: neg stored in-block AND live-out cross-block (rule (b) on the
 * FpNeg pack's own lanes). */
float v7_neg_liveout(const float *restrict a, float *restrict t, int c) {
    float x0 = -a[0], x1 = -a[1], x2 = -a[2], x3 = -a[3];
    t[0] = x0; t[1] = x1; t[2] = x2; t[3] = x3;
    if (c) {
        return x0 * x1;
    }
    return x2 + x3;
}

/* D4: f64 256-bit neg (vxorpd ymm with the 32-byte sign mask). */
void v7_neg_f64x4(const double *restrict a, double *restrict q) {
    q[0] = -a[0]; q[1] = -a[1]; q[2] = -a[2]; q[3] = -a[3];
}

/* ── Section E: memfold corners ─────────────────────────────────────── */

/* E1: f32x4 streamed fold (vaddps mem form). */
void v7_mf_f32(const float *restrict a, float *restrict q) {
    q[0] = a[0] * 2.0f; q[1] = a[1] * 2.0f; q[2] = a[2] * 2.0f;
    q[3] = a[3] * 2.0f;
}
/* E2: f64x2 streamed fold (vaddpd/vmulpd mem form). */
void v7_mf_f64(const double *restrict a, double *restrict q) {
    q[0] = a[0] + 1.5; q[1] = a[1] + 1.5;
}
/* E3: i16x8 streamed fold (vpaddw mem form, 128-bit). */
void v7_mf_i16(const int16_t *restrict a, int16_t *restrict q) {
    q[0] = a[0] + 300; q[1] = a[1] + 300; q[2] = a[2] + 300; q[3] = a[3] + 300;
    q[4] = a[4] + 300; q[5] = a[5] + 300; q[6] = a[6] + 300; q[7] = a[7] + 300;
}
/* E4: the fold in the args[0] (src1) position of a COMMUTATIVE op —
 * `k - a[i]`-style reversed streams must materialize (non-commutative),
 * `a[i] * k` must fold. */
void v7_mf_rev_sub(const int32_t *restrict a, int32_t *restrict q) {
    q[0] = 12345 - a[0]; q[1] = 12345 - a[1]; q[2] = 12345 - a[2];
    q[3] = 12345 - a[3];
}
/* E5: a shift consumer in the same block — the v6 ISA note: the VEX
 * immediate-shift encodings are register-only, so the adjacent load must
 * MATERIALISE (never fold into `vpslld $imm, MEM, xmm`). */
void v7_mf_shift(const int32_t *restrict a, int32_t *restrict q) {
    q[0] = a[0] << 3; q[1] = a[1] << 3; q[2] = a[2] << 3; q[3] = a[3] << 3;
    q[4] = a[4] << 3; q[5] = a[5] << 3; q[6] = a[6] << 3; q[7] = a[7] << 3;
}
/* E6: two-stream 128-bit with the farther load folding and the nearer
 * streaming, mixed widths of constant (immediate-range vs splat). */
void v7_mf_two128(const int32_t *restrict a, const int32_t *restrict b,
                  int32_t *restrict q) {
    q[0] = a[0] * b[0] + 1; q[1] = a[1] * b[1] + 1; q[2] = a[2] * b[2] + 1;
    q[3] = a[3] * b[3] + 1;
}

/* ── Section F: scheduling / pressure ───────────────────────────────── */

/* F1: splat-early under pressure — eight live streamed values plus two
 * constant splats; the values must be exact regardless of the schedule,
 * and the register-pressure answer must not spill incorrectly. */
void v7_pressure(const int32_t *restrict a, const int32_t *restrict b,
                 const int32_t *restrict c, int32_t *restrict q) {
    int32_t k = 0x1234;
    q[0] = a[0] + k; q[1] = a[1] + k; q[2] = a[2] + k; q[3] = a[3] + k;
    q[4] = b[0] - k; q[5] = b[1] - k; q[6] = b[2] - k; q[7] = b[3] - k;
    q[8] = c[0] ^ k; q[9] = c[1] ^ k; q[10] = c[2] ^ k; q[11] = c[3] ^ k;
}

/* F2: many-parameter prologue — the ParamRef prefix must survive the
 * splat-early scheduling (params keep their ABI homes). */
int32_t v7_manyparams(int32_t p0, int32_t p1, int32_t p2, int32_t p3,
                      int32_t p4, int32_t p5, const int32_t *restrict a,
                      int32_t *restrict q) {
    q[0] = a[0] + p0; q[1] = a[1] + p1; q[2] = a[2] + p2; q[3] = a[3] + p3;
    return p4 + p5;
}

/* F3: long live range — the packed value's extracts feed uses at the
 * very end of a long block. */
uint32_t v7_longlive(const uint32_t *restrict a, uint32_t *restrict t) {
    uint32_t x0 = a[0] + 9, x1 = a[1] + 9, x2 = a[2] + 9, x3 = a[3] + 9;
    t[0] = x0; t[1] = x1; t[2] = x2; t[3] = x3;
    uint32_t acc = 0;
    for (int i = 0; i < 64; i++) {
        acc += i * 2654435761u;
    }
    return acc + x0 + x1 * 2 + x2 * 3 + x3 * 4;
}


/* ── Section H: sub-word select demotion (C integer promotion) ────────
 *
 * `q[i] = a[i] < b[i] ? x : y` on i8/i16/u8/u16 arrays promotes the
 * compare and the arms to int; the store truncates back. The demotion
 * packs the select at the LANE width with a predicate remapped by the
 * promotion kind (zext+Slt -> Ult, sext+Slt -> Slt) — bit-identical to
 * trunc(promoted select) by construction. Adversarial lanes hit the
 * sign edges; the out-of-range constant compare must stay scalar. */

/* H1: i16 min (pminsw) with sign edges. */
void v7_demot_min_i16(const int16_t *restrict a, const int16_t *restrict b,
                      int16_t *restrict q) {
    q[0] = a[0] < b[0] ? a[0] : b[0]; q[1] = a[1] < b[1] ? a[1] : b[1];
    q[2] = a[2] < b[2] ? a[2] : b[2]; q[3] = a[3] < b[3] ? a[3] : b[3];
    q[4] = a[4] < b[4] ? a[4] : b[4]; q[5] = a[5] < b[5] ? a[5] : b[5];
    q[6] = a[6] < b[6] ? a[6] : b[6]; q[7] = a[7] < b[7] ? a[7] : b[7];
}
/* H2: u8 mixed-arm select (zext promotion -> unsigned byte compare). */
void v7_demot_sel_u8(const uint8_t *restrict a, const uint8_t *restrict b,
                     uint8_t *restrict q) {
    q[0] = a[0] < b[0] ? 7 : 3; q[1] = a[1] < b[1] ? 7 : 3;
    q[2] = a[2] < b[2] ? 7 : 3; q[3] = a[3] < b[3] ? 7 : 3;
    q[4] = a[4] < b[4] ? 7 : 3; q[5] = a[5] < b[5] ? 7 : 3;
    q[6] = a[6] < b[6] ? 7 : 3; q[7] = a[7] < b[7] ? 7 : 3;
    q[8] = a[8] < b[8] ? 7 : 3; q[9] = a[9] < b[9] ? 7 : 3;
    q[10] = a[10] < b[10] ? 7 : 3; q[11] = a[11] < b[11] ? 7 : 3;
    q[12] = a[12] < b[12] ? 7 : 3; q[13] = a[13] < b[13] ? 7 : 3;
    q[14] = a[14] < b[14] ? 7 : 3; q[15] = a[15] < b[15] ? 7 : 3;
}
/* H3: u16 unsigned compare select (zext + Slt -> Ult remap). */
void v7_demot_ult_u16(const uint16_t *restrict a, const uint16_t *restrict b,
                      uint16_t *restrict q) {
    q[0] = a[0] < b[0] ? a[0] + 11u : b[0] * 3u;
    q[1] = a[1] < b[1] ? a[1] + 11u : b[1] * 3u;
    q[2] = a[2] < b[2] ? a[2] + 11u : b[2] * 3u;
    q[3] = a[3] < b[3] ? a[3] + 11u : b[3] * 3u;
    q[4] = a[4] < b[4] ? a[4] + 11u : b[4] * 3u;
    q[5] = a[5] < b[5] ? a[5] + 11u : b[5] * 3u;
    q[6] = a[6] < b[6] ? a[6] + 11u : b[6] * 3u;
    q[7] = a[7] < b[7] ? a[7] + 11u : b[7] * 3u;
}
/* H4: i8 select (no signed byte min — the blendv path). */
void v7_demot_sel_i8(const int8_t *restrict a, const int8_t *restrict b,
                     int8_t *restrict q) {
    q[0] = a[0] < b[0] ? a[0] : -7; q[1] = a[1] < b[1] ? a[1] : -7;
    q[2] = a[2] < b[2] ? a[2] : -7; q[3] = a[3] < b[3] ? a[3] : -7;
    q[4] = a[4] < b[4] ? a[4] : -7; q[5] = a[5] < b[5] ? a[5] : -7;
    q[6] = a[6] < b[6] ? a[6] : -7; q[7] = a[7] < b[7] ? a[7] : -7;
    q[8] = a[8] < b[8] ? a[8] : -7; q[9] = a[9] < b[9] ? a[9] : -7;
    q[10] = a[10] < b[10] ? a[10] : -7; q[11] = a[11] < b[11] ? a[11] : -7;
    q[12] = a[12] < b[12] ? a[12] : -7; q[13] = a[13] < b[13] ? a[13] : -7;
    q[14] = a[14] < b[14] ? a[14] : -7; q[15] = a[15] < b[15] ? a[15] : -7;
}
/* H5: fitting constant compare on u8 (const inside the narrow range). */
void v7_demot_const_u8(const uint8_t *restrict a, uint8_t *restrict q) {
    q[0] = a[0] < 200u ? 1u : 9u; q[1] = a[1] < 200u ? 1u : 9u;
    q[2] = a[2] < 200u ? 1u : 9u; q[3] = a[3] < 200u ? 1u : 9u;
    q[4] = a[4] < 200u ? 1u : 9u; q[5] = a[5] < 200u ? 1u : 9u;
    q[6] = a[6] < 200u ? 1u : 9u; q[7] = a[7] < 200u ? 1u : 9u;
    q[8] = a[8] < 200u ? 1u : 9u; q[9] = a[9] < 200u ? 1u : 9u;
    q[10] = a[10] < 200u ? 1u : 9u; q[11] = a[11] < 200u ? 1u : 9u;
    q[12] = a[12] < 200u ? 1u : 9u; q[13] = a[13] < 200u ? 1u : 9u;
    q[14] = a[14] < 200u ? 1u : 9u; q[15] = a[15] < 200u ? 1u : 9u;
}
/* H6: OUT-OF-RANGE constant compare on i16 — the promoted compare is
 * always true (40000 > INT16_MAX), the demotion must reject and the
 * scalar values must be exact. */
void v7_demot_reject_const(const int16_t *restrict a, int16_t *restrict q) {
    q[0] = a[0] < 40000 ? 5 : 6; q[1] = a[1] < 40000 ? 5 : 6;
    q[2] = a[2] < 40000 ? 5 : 6; q[3] = a[3] < 40000 ? 5 : 6;
    q[4] = a[4] < 40000 ? 5 : 6; q[5] = a[5] < 40000 ? 5 : 6;
    q[6] = a[6] < 40000 ? 5 : 6; q[7] = a[7] < 40000 ? 5 : 6;
}

/* ── main ───────────────────────────────────────────────────────────── */

int main(void) {
    /* A: cross-block. */
    {
        uint32_t a[4] = {10, 20, 30, 40};
        CHECK(v7_x_selfloop(a, 5) == v7_x_selfloop(a, 5), "selfloop deterministic");
        /* spelling-twin reference computed by hand: acc adds x0-before
         * values 1,10,11,12,13 over 5 iterations; final x_i = a[i] + 4. */
        uint32_t want = (1u + 10u + 11u + 12u + 13u) + 14 + 24 + 34 + 44;
        CHECK(v7_x_selfloop(a, 5) == want, "selfloop value");
        uint32_t t[4];
        uint32_t r1 = v7_x_phi(a, t, 1);
        uint32_t r0 = v7_x_phi(a, t, 0);
        CHECK(r1 == ((17u ^ 27u) + (17u & 27u) + (37u ^ 47u)), "phi c=1");
        CHECK(r0 == ((37u | 47u) + (17u & 27u) + (37u ^ 47u)), "phi c=0");
        CHECK(t[0] == 17 && t[1] == 27 && t[2] == 37 && t[3] == 47, "phi stores");
        CHECK(v7_x_nested(a, t, 1, 1) == 30 + 60 * 2, "nested 1,1");
        CHECK(v7_x_nested(a, t, 1, 0) == 90 + 120 * 4, "nested 1,0");
        CHECK(v7_x_nested(a, t, 0, 1) == 0, "nested 0,x");
        uint32_t m1 = v7_x_mixed(a, t, 1);
        uint32_t m0 = v7_x_mixed(a, t, 0);
        /* x = {5,15,25,35}; inblock = (5+15)^(25+35) = 20^60; */
        CHECK(m1 == ((20u ^ 60u) + 5 * 35), "mixed c=1");
        CHECK(m0 == ((20u ^ 60u) + 15 + 25), "mixed c=0");
        uint64_t a64[4] = {100, 200, 300, 400};
        uint64_t t64[2];
        CHECK(v7_x_stored_liveout(a64, t64, 1) == (uint32_t)(300 * 3 + 700),
              "stored_liveout c=1");
        CHECK(v7_x_stored_liveout(a64, t64, 0) == (uint32_t)(300u ^ 700u),
              "stored_liveout c=0");
    }

    /* B: cmp+blendv corners. */
    {
        int8_t a8[16], b8[16], q8[16];
        for (int i = 0; i < 16; i++) {
            a8[i] = (int8_t)(i * 17 - 40);
            b8[i] = (int8_t)(40 - i * 17);
            q8[i] = 0;
        }
        v7_sel_i8(a8, b8, q8);
        int ok8 = 1;
        for (int i = 0; i < 16; i++) {
            int8_t want = a8[i] < b8[i] ? 7 : -7;
            if (q8[i] != want) ok8 = 0;
        }
        CHECK(ok8, "sel_i8");

        /* f64 <= with NaN lanes: C's <= is false on NaN → false arm. */
        double af[2] = {1.0, 0.0 / 0.0};
        double bf[2] = {2.0, 5.0};
        double qf[2];
        v7_fsel_f64(af, bf, qf);
        chkd(qf[0], 1.0, "fsel_f64 normal");
        chkd(qf[1], 5.0, "fsel_f64 nan-false-arm");

        /* mirrored >= on NaN: false on NaN → false arm (b verbatim —
         * bit-exact, no NaN arithmetic in the arms). */
        float ag[4] = {1.0f, 0.0f / 0.0f, -0.0f, 3.0f};
        float bg[4] = {2.0f, 5.0f, 0.0f, 0.0f / 0.0f};
        float qg[4];
        v7_fsel_ge(ag, bg, qg);
        chkf(qg[0], 2.0f, "fsel_ge lt");
        chkf(qg[1], 5.0f, "fsel_ge nan");
        /* -0.0 >= 0.0 is TRUE in C (== holds). */
        chkf(qg[2], -0.0f * 2.0f, "fsel_ge -0>=+0 true");
        /* 3.0 >= NaN is false → b (NaN) verbatim. */
        chkf(qg[3], bg[3], "fsel_ge gt-nan-rhs");

        /* unsigned > on sign-bit patterns: 0x80000000 > 0x7fffffff is
         * TRUE unsigned, FALSE signed. */
        uint32_t au[4] = {0x80000000u, 0x7fffffffu, 0xffffffffu, 1u};
        uint32_t bu[4] = {0x7fffffffu, 0x80000000u, 0u, 0xffffffffu};
        uint32_t qu[4], qs[4];
        v7_sel_ugt(au, bu, qu);
        v7_sel_sgt(au, bu, qs);
        CHECK(qu[0] == 0x11111111u, "ugt signbit");
        CHECK(qu[1] == 0xeeeeeeeeu, "ugt signbit rev");
        CHECK(qu[2] == 0x11111111u, "ugt max vs 0");
        CHECK(qu[3] == 0xeeeeeeeeu, "ugt 1 vs max");
        CHECK(qs[0] == 0xeeeeeeeeu, "sgt signbit (negative)");
        CHECK(qs[1] == 0x11111111u, "sgt signbit rev (positive)");
        CHECK(qs[2] == 0xeeeeeeeeu, "sgt -1 vs 0 false");
        CHECK(qs[3] == 0x11111111u, "sgt 1 vs -1 true");

        int32_t as[4] = {-5, 0, 7, 3};
        int32_t bs2[4] = {-3, 0, -7, 3};
        int32_t qs1[4], qr1[4];
        v7_sel_shared_cmp(as, bs2, qs1, qr1);
        CHECK(qs1[0] == 1 && qs1[1] == 2 && qs1[2] == 2 && qs1[3] == 2,
              "shared_cmp q");
        CHECK(qr1[0] == 10 && qr1[1] == 20 && qr1[2] == 20 && qr1[3] == 20,
              "shared_cmp r");

        float am[4] = {1.0f, 0.0f / 0.0f, -1.0f, 2.0f};
        float bm[4] = {2.0f, 3.0f, -3.0f, 0.0f / 0.0f};
        float qm[4];
        v7_fsel_mirror(am, bm, qm);
        chkf(qm[0], 1.0f, "mirror gt-false arm is a");
        chkf(qm[1], am[1], "mirror nan-lhs -> a verbatim");
        chkf(qm[2], -3.0f, "mirror a>b true -> b");
        chkf(qm[3], 2.0f, "mirror nan-rhs -> a");
    }

    /* C: int min/max corners. */
    {
        uint32_t a[4] = {0x80000000u, 5u, 0xffffffffu, 0u};
        uint32_t b[4] = {0x7fffffffu, 6u, 0x80000001u, 0xffffffffu};
        uint32_t q[4];
        v7_umin_signed_cmp(a, b, q);
        CHECK(q[0] == 0x80000000u, "umin_signed INT_MIN");
        CHECK(q[1] == 5u, "umin_signed small");
        CHECK(q[2] == 0x80000001u, "umin_signed -max wins");
        CHECK(q[3] == 0xffffffffu, "umin_signed 0 < -1 false -> b");

        int16_t a16[8], b16[8], q16[8];
        for (int i = 0; i < 8; i++) {
            a16[i] = (int16_t)(-30000 + i * 1234);
            b16[i] = (int16_t)(30000 - i * 1234);
        }
        v7_imin_i16x8(a16, b16, q16);
        int ok16 = 1;
        for (int i = 0; i < 8; i++) {
            int16_t want = a16[i] < b16[i] ? a16[i] : b16[i];
            if (q16[i] != want) ok16 = 0;
        }
        CHECK(ok16, "imin_i16x8");

        uint8_t a8[16], b8[16], q8[16];
        for (int i = 0; i < 16; i++) {
            a8[i] = (uint8_t)(i * 16);
            b8[i] = (uint8_t)(255 - i * 17);
        }
        v7_umin_u8x16(a8, b8, q8);
        int oku8 = 1;
        for (int i = 0; i < 16; i++) {
            uint8_t want = a8[i] < b8[i] ? a8[i] : b8[i];
            if (q8[i] != want) oku8 = 0;
        }
        CHECK(oku8, "umin_u8x16");

        int32_t ca[4] = {-100, 50, -50, 100};
        int32_t cb[4] = {100, -50, 50, -100};
        int32_t cc[4] = {0, 0, 0, 0};
        int32_t qc[4];
        v7_clamp(ca, cb, cc, qc);
        CHECK(qc[0] == 0 && qc[1] == 0 && qc[2] == 0 && qc[3] == 0, "clamp");
    }

    /* D: FP-Neg chains. */
    {
        float a[4] = {1.5f, -0.0f, 0.0f / 0.0f, -2.25f};
        float q[4];
        v7_negneg_f32(a, q);
        chkf(q[0], 1.5f, "negneg normal");
        chkf(q[1], -0.0f, "negneg -0 (double flip)");
        chkf(q[3], -2.25f, "negneg negative");
        /* NaN: -(-nan) flips the sign twice → original sign bit. */
        CHECK((bitsf(q[2]) ^ bitsf(0.0f / 0.0f)) == 0x80000000u ||
              (bitsf(q[2]) == bitsf(0.0f / 0.0f)),
              "negneg nan sign");
        float b[4] = {1.0f, 2.0f, 3.0f, 4.0f};
        v7_neg_arith(a, b, q);
        chkf(q[0], -1.5f + 1.0f, "neg_arith 0");
        chkf(q[3], 2.25f + 4.0f, "neg_arith 3");
        float t[4];
        float r1 = v7_neg_liveout(a, t, 1);
        float r0 = v7_neg_liveout(a, t, 0);
        /* x0 = -1.5, x1 = -(-0.0) = +0.0: r1 = -1.5 * +0.0 = -0.0. */
        CHECK(bitsf(r1) == 0x80000000u, "neg_liveout c=1 -0.0 product");
        /* r0 = -NaN + 2.25 = NaN (first-op NaN propagates): verify
         * NaN-ness bit-exactly by class, payload unspecified by C. */
        {
            uint32_t b0 = bitsf(r0);
            CHECK((b0 & 0x7f800000u) == 0x7f800000u && (b0 & 0x007fffffu) != 0u,
                  "neg_liveout c=0 nan class");
        }
        double ad[4] = {1.5, -0.0, 3.25, -7.0};
        double qd[4];
        v7_neg_f64x4(ad, qd);
        chkd(qd[0], -1.5, "neg_f64x4 0");
        chkd(qd[1], 0.0, "neg_f64x4 -0");
        chkd(qd[2], -3.25, "neg_f64x4 2");
        chkd(qd[3], 7.0, "neg_f64x4 3");
    }

    /* E: memfold corners. */
    {
        float af[4] = {1.0f, -2.5f, 0.0f / 0.0f, 4.0f};
        float qf[4];
        v7_mf_f32(af, qf);
        chkf(qf[0], 2.0f, "mf_f32 0");
        chkf(qf[1], -5.0f, "mf_f32 1");
        chkf(qf[3], 8.0f, "mf_f32 3");
        double ad[2] = {1.25, -3.0};
        double qd[2];
        v7_mf_f64(ad, qd);
        chkd(qd[0], 2.75, "mf_f64 0");
        chkd(qd[1], -1.5, "mf_f64 1");
        int16_t a16[8], q16[8];
        for (int i = 0; i < 8; i++) a16[i] = (int16_t)(-3000 + i * 900);
        v7_mf_i16(a16, q16);
        int ok16 = 1;
        for (int i = 0; i < 8; i++) {
            int32_t wide = (int32_t)a16[i] + 300;
            if (q16[i] != (int16_t)wide) ok16 = 0;
        }
        CHECK(ok16, "mf_i16 wrapping");
        int32_t ai[4] = {1, 2, 3, 4}, qi[4];
        v7_mf_rev_sub(ai, qi);
        CHECK(qi[0] == 12344 && qi[3] == 12341, "mf_rev_sub");
        int32_t as[8] = {1, -2, 3, -4, 5, -6, 7, -8}, qs[8];
        v7_mf_shift(as, qs);
        CHECK(qs[0] == 8 && qs[1] == -16 && qs[2] == 24 && qs[3] == -32,
              "mf_shift low");
        CHECK(qs[4] == 40 && qs[5] == -48 && qs[6] == 56 && qs[7] == -64,
              "mf_shift high");
        int32_t a2[4] = {2, 3, 4, 5}, b2[4] = {10, 20, 30, 40};
        v7_mf_two128(a2, b2, qi);
        CHECK(qi[0] == 21 && qi[1] == 61 && qi[2] == 121 && qi[3] == 201,
              "mf_two128");
    }

    /* F: scheduling / pressure. */
    {
        int32_t a[4] = {1, 2, 3, 4}, b[4] = {5, 6, 7, 8}, c[4] = {9, 10, 11, 12};
        int32_t q[12];
        v7_pressure(a, b, c, q);
        int ok = 1;
        for (int i = 0; i < 4; i++) {
            if (q[i] != a[i] + 0x1234) ok = 0;
            if (q[4 + i] != b[i] - 0x1234) ok = 0;
            if (q[8 + i] != (c[i] ^ 0x1234)) ok = 0;
        }
        CHECK(ok, "pressure");
        int32_t qp[4];
        int32_t r = v7_manyparams(1, 2, 3, 4, 5, 6, a, qp);
        CHECK(r == 11, "manyparams ret");
        CHECK(qp[0] == 2 && qp[1] == 4 && qp[2] == 6 && qp[3] == 8,
              "manyparams q");
        uint32_t au[4] = {100, 200, 300, 400};
        uint32_t tu[4];
        uint32_t ll = v7_longlive(au, tu);
        uint32_t acc = 0;
        for (int i = 0; i < 64; i++) acc += (uint32_t)(i * 2654435761u);
        CHECK(ll == acc + 109 + 209 * 2 + 309 * 3 + 409 * 4, "longlive");
    }


    /* H: sub-word select demotion. */
    {
        int16_t a16[8] = {-32768, 32767, -1, 0, 1, -32768, 32767, 12345};
        int16_t b16[8] = {32767, -32768, 0, -1, -2, -32768, 32767, -12345};
        int16_t q16[8];
        v7_demot_min_i16(a16, b16, q16);
        int okm = 1;
        for (int i = 0; i < 8; i++) {
            int16_t want = a16[i] < b16[i] ? a16[i] : b16[i];
            if (q16[i] != want) okm = 0;
        }
        CHECK(okm, "demot_min_i16");

        uint8_t au8[16], qu8[16];
        for (int i = 0; i < 16; i++) au8[i] = (uint8_t)(i * 37 + 200);
        uint8_t bu8[16];
        for (int i = 0; i < 16; i++) bu8[i] = (uint8_t)(255 - i * 29);
        v7_demot_sel_u8(au8, bu8, qu8);
        int oku = 1;
        for (int i = 0; i < 16; i++) {
            uint8_t want = au8[i] < bu8[i] ? 7 : 3;
            if (qu8[i] != want) oku = 0;
        }
        CHECK(oku, "demot_sel_u8");

        uint16_t au16[8] = {0, 1, 0x8000, 0xffff, 0x7fff, 42, 0x8000, 0};
        uint16_t bu16[8] = {0, 0, 0x7fff, 0x8000, 0x8000, 42, 0xffff, 1};
        uint16_t qu16[8];
        v7_demot_ult_u16(au16, bu16, qu16);
        int okw = 1;
        for (int i = 0; i < 8; i++) {
            uint16_t want = au16[i] < bu16[i] ? (uint16_t)(au16[i] + 11u)
                                              : (uint16_t)(bu16[i] * 3u);
            if (qu16[i] != want) okw = 0;
        }
        CHECK(okw, "demot_ult_u16");

        int8_t ai8[16], qi8[16];
        for (int i = 0; i < 16; i++) ai8[i] = (int8_t)(i * 23 - 100);
        int8_t bi8[16];
        for (int i = 0; i < 16; i++) bi8[i] = (int8_t)(100 - i * 23);
        v7_demot_sel_i8(ai8, bi8, qi8);
        int ok8 = 1;
        for (int i = 0; i < 16; i++) {
            int8_t want = ai8[i] < bi8[i] ? ai8[i] : -7;
            if (qi8[i] != want) ok8 = 0;
        }
        CHECK(ok8, "demot_sel_i8");

        uint8_t ac8[16], qc8[16];
        for (int i = 0; i < 16; i++) ac8[i] = (uint8_t)(i * 16 + 190);
        v7_demot_const_u8(ac8, qc8);
        int okc = 1;
        for (int i = 0; i < 16; i++) {
            uint8_t want = ac8[i] < 200u ? 1u : 9u;
            if (qc8[i] != want) okc = 0;
        }
        CHECK(okc, "demot_const_u8");

        int16_t ar16[8] = {-32768, 32767, 0, -1, 1, 12345, -12345, 32767};
        int16_t qr16[8];
        v7_demot_reject_const(ar16, qr16);
        int okr = 1;
        for (int i = 0; i < 8; i++) {
            if (qr16[i] != 5) okr = 0; /* a < 40000 is always true */
        }
        CHECK(okr, "demot_reject_const");
    }

    if (fails == 0) {
        printf("bb_slp_v7: all pass (0 fails)\n");
    } else {
        printf("bb_slp_v7: %d FAILS\n", fails);
    }
    return fails != 0;
}
