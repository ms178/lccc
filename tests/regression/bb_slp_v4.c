/*
 * BB-SLP v4 contracts: affine window addressing, rule-(d) disjointness
 * escape, and the extended lane-extract families.
 *
 * Every shape here pins a v4 defect or coverage gap:
 *
 *  - v4_affine_window_*: `a[i-1..i+k]` indexed straight-line code never
 *    vectorized at all: the canonical pre-SLP offset shapes are
 *    `Add(x,x)` (simplify's Mul(x,2) canonicalization), `Shl(x,k)`
 *    (Mul(x,2^k)), `Sub(v,k)` (the `i-k` index), and widening `Cast`s —
 *    NONE of which the old one-level `Add(var, Const)` recognizer
 *    understood. The stores must now emit ONE vector load + ONE vector
 *    store (the movdqu pair) for every lane type with a family.
 *  - v4_extract_f32 / _i16 / _i32x8 / _f32x8 / _i16x16: a seed lane
 *    with an EXTERNAL use requires a lane extract. The F32x4, F32x8,
 *    I32x8, I16x8, and I16x16 families had NO extract intrinsic, so
 *    ANY externally-used lane rejected the whole seed. Runtime checks
 *    the extracted value AND the stored lanes (the 4-byte movss extract
 *    discipline; the pextrw halfword; the 256-bit half staging for
 *    lanes >= half).
 *  - v4_rule_d_disjoint: an interleaved access to a PROVABLY DISJOINT
 *    object (restrict param vs. plain global) between seed stores must
 *    no longer reject the seed — the batched commit is unobservable to
 *    it. GCC/Clang apply the same reasoning.
 *  - v4_rule_d_samestream: an interleaved write on the SAME stream but
 *    OUTSIDE the seed byte range (q[5] between q[0..3] seeds) is safe —
 *    the exact byte-range test must let the seed fire and the write
 *    survive (q[5] keeps its value).
 *  - v4_rule_d_alias: an interleaved access through a MAY-ALIAS pointer
 *    (loaded from memory, no restrict contract) must keep the seed
 *    REJECTED — the scalar order preserves the aliased write. Runtime
 *    pins the aliased value (a vectorized store would clobber it).
 */
#include <stdio.h>
#include <string.h>

#if defined(NO_MAIN)
/* codegen-only translation unit: same bodies, no main. */
#else
static int fails = 0;
#define CHECK(cond, msg) \
    do { if (!(cond)) { printf("FAIL: %s\n", msg); fails++; } } while (0)
#endif

/* ── Affine window shapes (the Add(x,x)/Shl/Sub/Cast recognition) ── */

void v4_affine_window_i32(const int *restrict a, int *restrict q, int i) {
    q[0] = a[i - 1];
    q[1] = a[i];
    q[2] = a[i + 1];
    q[3] = a[i + 2];
}

void v4_affine_window_f64(const double *restrict a, double *restrict q, int i) {
    q[0] = a[i - 1];
    q[1] = a[i];
    q[2] = a[i + 1];
    q[3] = a[i + 2];
}

void v4_affine_window_f32(const float *restrict a, float *restrict q, int i) {
    q[0] = a[i - 1];
    q[1] = a[i];
    q[2] = a[i + 1];
    q[3] = a[i + 2];
}

/* sizeof(long)=8: the scale arrives as Shl(x,3) (Mul(x,8) canonicalized). */
void v4_affine_shl_i64(const long *restrict a, long *restrict q, long i) {
    q[0] = a[i];
    q[1] = a[i + 1];
    q[2] = a[i + 2];
    q[3] = a[i + 3];
}

/* 8 halfword lanes: affine + the I16x8 family. */
void v4_affine_window_i16(const short *restrict a, short *restrict q, int i) {
    q[0] = a[i - 1];
    q[1] = a[i];
    q[2] = a[i + 1];
    q[3] = a[i + 2];
    q[4] = a[i + 3];
    q[5] = a[i + 4];
    q[6] = a[i + 5];
    q[7] = a[i + 6];
}

/* ── Lane-extract families ─────────────────────────────────────────── */

float v4_extract_f32(const float *restrict a, float *restrict q) {
    float x0 = a[0] + 1.0f;
    float x1 = a[1] + 1.0f;
    float x2 = a[2] + 1.0f;
    float x3 = a[3] + 1.0f;
    q[0] = x0;
    q[1] = x1;
    q[2] = x2;
    q[3] = x3;
    return x2; /* external use: F32x4 lane-2 extract */
}

short v4_extract_i16(const short *restrict a, short *restrict q) {
    short x0 = a[0] + 1;
    short x1 = a[1] + 1;
    short x2 = a[2] + 1;
    short x3 = a[3] + 1;
    short x4 = a[4] + 1;
    short x5 = a[5] + 1;
    short x6 = a[6] + 1;
    short x7 = a[7] + 1;
    q[0] = x0;
    q[1] = x1;
    q[2] = x2;
    q[3] = x3;
    q[4] = x4;
    q[5] = x5;
    q[6] = x6;
    q[7] = x7;
    return x5; /* external use: I16x8 lane-5 extract (pextrw) */
}

int v4_extract_i32x8(const int *restrict a, int *restrict q) {
    int x0 = a[0] + 1;
    int x1 = a[1] + 1;
    int x2 = a[2] + 1;
    int x3 = a[3] + 1;
    int x4 = a[4] + 1;
    int x5 = a[5] + 1;
    int x6 = a[6] + 1;
    int x7 = a[7] + 1;
    q[0] = x0;
    q[1] = x1;
    q[2] = x2;
    q[3] = x3;
    q[4] = x4;
    q[5] = x5;
    q[6] = x6;
    q[7] = x7;
    return x5; /* external use: I32x8 lane-5 extract (HIGH half staging) */
}

float v4_extract_f32x8(const float *restrict a, float *restrict q) {
    float x0 = a[0] + 1.0f;
    float x1 = a[1] + 1.0f;
    float x2 = a[2] + 1.0f;
    float x3 = a[3] + 1.0f;
    float x4 = a[4] + 1.0f;
    float x5 = a[5] + 1.0f;
    float x6 = a[6] + 1.0f;
    float x7 = a[7] + 1.0f;
    q[0] = x0;
    q[1] = x1;
    q[2] = x2;
    q[3] = x3;
    q[4] = x4;
    q[5] = x5;
    q[6] = x6;
    q[7] = x7;
    return x6; /* external use: F32x8 lane-6 extract (HIGH half) */
}

short v4_extract_i16x16(const short *restrict a, short *restrict q) {
    short x[16];
    for (int k = 0; k < 16; k++)
        x[k] = a[k] + 1;
    for (int k = 0; k < 16; k++)
        q[k] = x[k];
    return x[11]; /* external use: I16x16 lane-11 (HIGH half) */
}

/* ── Rule-(d) escape shapes ────────────────────────────────────────── */

static int v4_acc;
static int v4_other[2];

void v4_rule_d_disjoint(const int *restrict a, int *restrict q) {
    q[0] = a[0];
    v4_acc += v4_other[0];
    q[1] = a[1];
    v4_acc += v4_other[1];
    q[2] = a[2];
    q[3] = a[3];
}

void v4_rule_d_samestream(const int *restrict a, int *restrict q) {
    q[0] = a[0];
    q[5] = 9; /* same stream as the seeds, outside [0,16): safe */
    q[1] = a[1];
    q[2] = a[2];
    q[3] = a[3];
}

void v4_rule_d_alias(int *q, int **rp) {
    q[0] = 1;
    **rp += 5; /* may alias q (loaded pointer, no contract) */
    q[1] = 2;
    q[2] = 3;
    q[3] = 4;
}

/* ── Review follow-up coverage (F1/F2/F5 contracts) ────────────────── */

/* 16 halfword lanes of mixed bitwise ops with constant splats: pins the
 * word-family bitwise arms end to end (the SLP-level surface the loop
 * vectorizer does not yet emit — recorded follow-up) and the broadcast
 * registration classes the ops flow through (F2). */
void v4_word_bitwise16(const unsigned short *restrict m,
                       const unsigned short *restrict k,
                       unsigned short *restrict q) {
    q[0] = (m[0] & 0x00ffu) | (k[0] ^ 0xff00u);
    q[1] = (m[1] & 0x00ffu) | (k[1] ^ 0xff00u);
    q[2] = (m[2] & 0x00ffu) | (k[2] ^ 0xff00u);
    q[3] = (m[3] & 0x00ffu) | (k[3] ^ 0xff00u);
    q[4] = (m[4] & 0x00ffu) | (k[4] ^ 0xff00u);
    q[5] = (m[5] & 0x00ffu) | (k[5] ^ 0xff00u);
    q[6] = (m[6] & 0x00ffu) | (k[6] ^ 0xff00u);
    q[7] = (m[7] & 0x00ffu) | (k[7] ^ 0xff00u);
    q[8] = (m[8] & 0x00ffu) | (k[8] ^ 0xff00u);
    q[9] = (m[9] & 0x00ffu) | (k[9] ^ 0xff00u);
    q[10] = (m[10] & 0x00ffu) | (k[10] ^ 0xff00u);
    q[11] = (m[11] & 0x00ffu) | (k[11] ^ 0xff00u);
    q[12] = (m[12] & 0x00ffu) | (k[12] ^ 0xff00u);
    q[13] = (m[13] & 0x00ffu) | (k[13] ^ 0xff00u);
    q[14] = (m[14] & 0x00ffu) | (k[14] ^ 0xff00u);
    q[15] = (m[15] & 0x00ffu) | (k[15] ^ 0xff00u);
}

/* 16 halfword zero stores: the I16x16 zero splat must take the one-
 * instruction vpxor path (F5), not the xorl+movd+vpbroadcastw chain. */
void v4_zstore_i16(short *restrict q) {
    q[0] = 0; q[1] = 0; q[2] = 0; q[3] = 0;
    q[4] = 0; q[5] = 0; q[6] = 0; q[7] = 0;
    q[8] = 0; q[9] = 0; q[10] = 0; q[11] = 0;
    q[12] = 0; q[13] = 0; q[14] = 0; q[15] = 0;
}

/* 16 halfword all-ones stores: the pcmpeqd self-compare on the word
 * family (F5 sibling contract — one vpcmpeqd, one store). */
void v4_ones_i16(short *restrict q) {
    q[0] = -1; q[1] = -1; q[2] = -1; q[3] = -1;
    q[4] = -1; q[5] = -1; q[6] = -1; q[7] = -1;
    q[8] = -1; q[9] = -1; q[10] = -1; q[11] = -1;
    q[12] = -1; q[13] = -1; q[14] = -1; q[15] = -1;
}

#if !defined(NO_MAIN)
int main(void) {
    {
        int a[8] = {10, 11, 12, 13, 14, 15, 16, 17}, q[4];
        v4_affine_window_i32(a + 1, q, 1);
        CHECK(q[0] == 11 && q[1] == 12 && q[2] == 13 && q[3] == 14,
              "affine_window_i32 values");
        double da[8] = {1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5, 8.5}, dq[4];
        v4_affine_window_f64(da + 1, dq, 1);
        CHECK(dq[0] == 2.5 && dq[1] == 3.5 && dq[2] == 4.5 && dq[3] == 5.5,
              "affine_window_f64 values");
        float fa[8] = {1.0f, 2.0f, 3.0f, 4.0f, 5.0f, 6.0f, 7.0f, 8.0f}, fq[4];
        v4_affine_window_f32(fa + 1, fq, 1);
        CHECK(fq[0] == 2.0f && fq[1] == 3.0f && fq[2] == 4.0f && fq[3] == 5.0f,
              "affine_window_f32 values");
        long la[8] = {100, 201, 302, 403, 504, 605, 706, 807}, lq[4];
        v4_affine_shl_i64(la, lq, 2);
        CHECK(lq[0] == 302 && lq[1] == 403 && lq[2] == 504 && lq[3] == 605,
              "affine_shl_i64 values (Shl scale path)");
        short sa[10] = {-1, -2, -3, -4, -5, -6, -7, -8, -9, -10}, sq[8];
        v4_affine_window_i16(sa + 1, sq, 1);
        CHECK(sq[0] == -2 && sq[1] == -3 && sq[2] == -4 && sq[3] == -5
                  && sq[4] == -6 && sq[5] == -7 && sq[6] == -8 && sq[7] == -9,
              "affine_window_i16 values");
    }

    {
        float a[4] = {1.0f, 2.0f, 3.0f, 4.0f}, q[4];
        float r = v4_extract_f32(a, q);
        CHECK(r == 4.0f, "extract_f32 lane 2 (3.0 + 1.0)");
        CHECK(q[0] == 2.0f && q[1] == 3.0f && q[2] == 4.0f && q[3] == 5.0f,
              "extract_f32 stored lanes");
    }
    {
        short a[8] = {10, 20, 30, 40, 50, 60, 70, 80}, q[8];
        short r = v4_extract_i16(a, q);
        CHECK(r == 61, "extract_i16 lane 5 (60 + 1)");
        CHECK(q[0] == 11 && q[7] == 81, "extract_i16 stored lanes");
    }
    {
        int a[8] = {1, 2, 3, 4, 5, 6, 7, 8}, q[8];
        int r = v4_extract_i32x8(a, q);
        CHECK(r == 7, "extract_i32x8 high-half lane 5 (6 + 1)");
        CHECK(q[0] == 2 && q[4] == 6 && q[7] == 9, "extract_i32x8 stored lanes");
    }
    {
        float a[8] = {1, 2, 3, 4, 5, 6, 7, 8}, q[8];
        float r = v4_extract_f32x8(a, q);
        CHECK(r == 8.0f, "extract_f32x8 high-half lane 6 (7 + 1)");
        CHECK(q[0] == 2.0f && q[4] == 6.0f && q[7] == 9.0f,
              "extract_f32x8 stored lanes");
    }
    {
        short a[16], q[16];
        for (int k = 0; k < 16; k++)
            a[k] = (short)(k * 10);
        short r = v4_extract_i16x16(a, q);
        CHECK(r == 111, "extract_i16x16 high-half lane 11 (110 + 1)");
        CHECK(q[0] == 1 && q[15] == 151, "extract_i16x16 stored lanes");
    }

    {
        int a[4] = {7, 8, 9, 10}, q[4];
        v4_acc = 0;
        v4_other[0] = 100;
        v4_other[1] = 200;
        v4_rule_d_disjoint(a, q);
        CHECK(q[0] == 7 && q[1] == 8 && q[2] == 9 && q[3] == 10,
              "rule_d_disjoint stored lanes");
        CHECK(v4_acc == 300, "rule_d_disjoint interleaved reads survived");
    }
    {
        int a[8] = {4, 5, 6, 7, 8, 9, 10, 11}, q[8];
        memset(q, 0, sizeof q);
        v4_rule_d_samestream(a, q);
        CHECK(q[0] == 4 && q[1] == 5 && q[2] == 6 && q[3] == 7,
              "rule_d_samestream seed lanes");
        CHECK(q[5] == 9, "rule_d_samestream out-of-range write survived");
    }
    {
        int q[4];
        int *rp = &q[0];
        memset(q, 0, sizeof q);
        v4_rule_d_alias(q, &rp);
        /* Scalar order: q[0]=1; *rp += 5 -> q[0]=6; q[1..3]=2,3,4. */
        CHECK(q[0] == 6, "rule_d_alias: the aliased write is preserved");
        CHECK(q[1] == 2 && q[2] == 3 && q[3] == 4, "rule_d_alias other lanes");
    }

    {
        unsigned short m[16], k[16], q[16];
        for (int i = 0; i < 16; i++) {
            m[i] = (unsigned short)(0x1234u + i);
            k[i] = (unsigned short)(0x5678u + i * 7);
        }
        v4_word_bitwise16(m, k, q);
        int bad = 0;
        for (int i = 0; i < 16; i++) {
            unsigned short ref =
                (unsigned short)((m[i] & 0x00ffu) | (k[i] ^ 0xff00u));
            if (q[i] != ref)
                bad++;
        }
        CHECK(bad == 0, "word_bitwise16 values (and/or/xor halfword maps)");
    }
    {
        short q[16];
        memset(q, 0x5a, sizeof q);
        v4_zstore_i16(q);
        int bad = 0;
        for (int i = 0; i < 16; i++)
            if (q[i] != 0)
                bad++;
        CHECK(bad == 0, "zstore_i16 values (vpxor zero splat)");
    }
    {
        short q[16];
        memset(q, 0, sizeof q);
        v4_ones_i16(q);
        int bad = 0;
        for (int i = 0; i < 16; i++)
            if (q[i] != -1)
                bad++;
        CHECK(bad == 0, "ones_i16 values (vpcmpeqd ones splat)");
    }

    printf("bb_slp_v4: all pass (%d fails)\n", fails);
    return fails != 0;
}
#endif
