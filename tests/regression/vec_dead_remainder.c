/*
 * vec_dead_remainder.c — correctness corpus for ZERO-REM-1 (provably-dead
 * vectorizer remainder loops).
 *
 * WHY THIS FILE EXISTS
 * --------------------
 * The map vectorizer emits a packed loop plus a SCALAR MIRROR that runs the
 * `n % W` tail elements the packed body cannot cover.  When the trip count is
 * a compile-time constant the packed body covers exactly (`n % W == 0`) that
 * mirror cannot execute a single iteration, but it used to be emitted anyway:
 * a guard, a resume-index computation, two materialised stream pointers and a
 * whole scalar loop body.  `transform_map_vector` now omits it.
 *
 * The mirror is not only a tail loop — it is also the FALLBACK TARGET of the
 * runtime dependence guards (a stream whose object root may alias the
 * destination re-enters the loop scalar-wise through the mirror).  Dropping it
 * is therefore only legal when
 *
 *   1. the trip count is a compile-time constant multiple of the packed width,
 *   2. no stream is alias-guarded (`pattern.guarded_streams` is empty), and
 *   3. every use of the scalar counter that escapes the loop is re-pointed at
 *      the trip count, which the transform materialises in the preheader.
 *
 * This file pins all three, and pins the cases where the mirror MUST stay
 * (non-multiple trip counts, may-alias streams, dynamic trip counts).  Every
 * shape prints into one rolling checksum so a wrong answer is a wrong line,
 * not a silently passing run.
 *
 * Element counts are chosen so that they are exact multiples of the packed
 * width under BOTH baselines the gate compiles for: x86-64-v3 (AVX2, 32-byte
 * vectors) and x86-64 (SSE2, 16-byte vectors).  `u32`: 32 lanes cover 8 (AVX2)
 * and 4 (SSE2) elements per step, and 32 % 8 == 32 % 4 == 0.  Same reasoning
 * for u8/64, u16/32, f32/32, f64/8 and u64/8 (the I64 map deliberately lowers
 * to two lanes, so 8 % 2 == 8 % 4 == 0 as well).
 */
#include <stdio.h>

typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned int u32;
typedef unsigned long long u64;

/* ── rolling checksum ──────────────────────────────────────────────────────
 * A single 64-bit mixer over every produced byte: one wrong element anywhere
 * in any shape changes the printed line.  Mixing bytes (not words) also
 * catches a tail that writes the wrong *number* of elements with the right
 * values, because the untouched bytes stay at their sentinel fill.          */
static u64 state = 0x243F6A8885A308D3ULL;

static void mix(u64 v) {
    state ^= v + 0x9E3779B97F4A7C15ULL + (state << 6) + (state >> 2);
}

static void mixbuf(const void *p, unsigned long n, unsigned long stride) {
    const unsigned char *b = (const unsigned char *)p;
    unsigned long i;
    for (i = 0; i < n; i++) mix((u64)b[i * stride]);
}

static u64 rng = 0x123456789abcdefULL;
static u32 next_rng(void) {
    rng ^= rng << 13;
    rng ^= rng >> 7;
    rng ^= rng << 17;
    return (u32)(rng >> 11);
}

#define DIM 128

static u8 a8[DIM], b8[DIM];
static u16 a16[DIM], b16[DIM];
static u32 a32[DIM], b32[DIM], c32[DIM];
static u64 a64[DIM], b64[DIM];
static int ai32[DIM], bi32[DIM];
static float af32[DIM], bf32[DIM];
static double af64[DIM], bf64[DIM];

static void fill(void) {
    int i;
    for (i = 0; i < DIM; i++) {
        u32 v = next_rng();
        a8[i] = (u8)v;
        b8[i] = (u8)(v >> 8);
        a16[i] = (u16)v;
        b16[i] = (u16)(v >> 8);
        a32[i] = v;
        b32[i] = v >> 3;
        c32[i] = v >> 5;
        a64[i] = ((u64)v << 32) | (u64)(v ^ 0x5a5a5a5au);
        b64[i] = ((u64)v << 17) ^ 0x123456789abcdefULL;
        /* Keep the signed shape far from INT_MAX: `s[i] * 3 + 1` must not
         * overflow, or the "correct" answer itself would be undefined. */
        ai32[i] = (int)(v % 2000u) - 1000;
        bi32[i] = (int)((v >> 4) % 2000u) - 1000;
        af32[i] = (float)(v % 1000u) * 0.5f;
        bf32[i] = (float)((v >> 6) % 1000u) * 0.25f;
        af64[i] = (double)(v % 1000u) * 0.5;
        bf64[i] = (double)((v >> 6) % 1000u) * 0.25;
    }
}

/* ── 1. exact multiples: the mirror is dead and must be omitted ─────────── */

void map_u32_32(u32 *__restrict d, const u32 *__restrict s) {
    int i;
    for (i = 0; i < 32; i++) d[i] = s[i] * 3u + 1u;
}

void map_u8_64(u8 *__restrict d, const u8 *__restrict s) {
    int i;
    for (i = 0; i < 64; i++) d[i] = (u8)(s[i] + 7u);
}

void map_u16_32(u16 *__restrict d, const u16 *__restrict s) {
    int i;
    for (i = 0; i < 32; i++) d[i] = (u16)(s[i] * 5u + 2u);
}

void map_u64_8(u64 *__restrict d, const u64 *__restrict s) {
    int i;
    for (i = 0; i < 8; i++) d[i] = s[i] * 3ull + 1ull;
}

void map_i32_32(int *__restrict d, const int *__restrict s) {
    int i;
    for (i = 0; i < 32; i++) d[i] = s[i] * 3 + 1;
}

void map_f32_32(float *__restrict d, const float *__restrict s) {
    int i;
    for (i = 0; i < 32; i++) d[i] = s[i] * 3.0f + 1.0f;
}

void map_f64_8(double *__restrict d, const double *__restrict s) {
    int i;
    for (i = 0; i < 8; i++) d[i] = s[i] * 3.0 + 1.0;
}

/* Two source streams, both advanced by the shared byte induction variable. */
void map2_u32_32(u32 *__restrict d, const u32 *__restrict a,
                 const u32 *__restrict b) {
    int i;
    for (i = 0; i < 32; i++) d[i] = a[i] * 3u + b[i];
}

/* ── 2. non-multiples: the scalar tail must still run ───────────────────── */

void map_u32_33(u32 *__restrict d, const u32 *__restrict s) {
    int i;
    for (i = 0; i < 33; i++) d[i] = s[i] * 3u + 1u;
}

void map_u8_67(u8 *__restrict d, const u8 *__restrict s) {
    int i;
    for (i = 0; i < 67; i++) d[i] = (u8)(s[i] + 7u);
}

void map_u16_35(u16 *__restrict d, const u16 *__restrict s) {
    int i;
    for (i = 0; i < 35; i++) d[i] = (u16)(s[i] * 5u + 2u);
}

void map_u64_9(u64 *__restrict d, const u64 *__restrict s) {
    int i;
    for (i = 0; i < 9; i++) d[i] = s[i] * 3ull + 1ull;
}

void map_f32_35(float *__restrict d, const float *__restrict s) {
    int i;
    for (i = 0; i < 35; i++) d[i] = s[i] * 3.0f + 1.0f;
}

/* ── 3. the counter escapes: materialise the trip count, do not leak the
 *      vector counter (which counts PACKED iterations) ─────────────────── */

int map_esc_u32(u32 *__restrict d, const u32 *__restrict s) {
    int i;
    for (i = 0; i < 32; i++) d[i] = s[i] + 11u;
    return i;
}

int map_esc_u8(u8 *__restrict d, const u8 *__restrict s) {
    int i;
    for (i = 0; i < 64; i++) d[i] = (u8)(s[i] + 11u);
    return i;
}

int map_esc_u64(u64 *__restrict d, const u64 *__restrict s) {
    int i;
    for (i = 0; i < 8; i++) d[i] = s[i] + 11ull;
    return i;
}

/* Long counter type: the materialised trip count must be narrowed to the
 * counter's own type, not to the byte IV's I64. */
long long map_esc_i64(double *__restrict d, const double *__restrict s) {
    long long i;
    for (i = 0; i < 8; i++) d[i] = s[i] + 11.0;
    return i;
}

/* ── 4. may-alias streams: the runtime guard falls back INTO the mirror, so
 *      the mirror must be kept even for an exact-multiple trip count ─────── */

void map_alias_u32(u32 *d, const u32 *s) {
    int i;
    for (i = 0; i < 32; i++) d[i] = s[i] + 5u;
}

/* Fixed dependence distance of one element in each direction. */
void map_alias_fwd(u32 *d, const u32 *s) {
    int i;
    for (i = 0; i < 32; i++) d[i] = s[i + 1] + 5u;
}

void map_alias_bwd(u32 *d, const u32 *s) {
    int i;
    for (i = 0; i < 32; i++) d[i + 1] = s[i] + 5u;
}

/* ── 5. dynamic trip count: no compile-time multiple is provable ────────── */

void map_dyn_u32(u32 *__restrict d, const u32 *__restrict s, int n) {
    int i;
    for (i = 0; i < n; i++) d[i] = s[i] * 3u + 1u;
}

/* ── 6. nested loops: the inner constant-trip map is the vectorized one ─── */

void map_nested_u32(u32 *__restrict d, const u32 *__restrict s, int rows) {
    int r, i;
    for (r = 0; r < rows; r++)
        for (i = 0; i < 32; i++) d[r * 32 + i] = s[r * 32 + i] + 13u;
}

int main(void) {
    int k;

    fill();

    /* 1 — exact multiples (mirror dead). */
    for (k = 0; k < 4; k++) {
        map_u32_32(c32, a32 + k);
        mixbuf(c32, 32, sizeof(u32));
    }
    for (k = 0; k < 2; k++) {
        map_u8_64(b8, a8 + k);
        mixbuf(b8, 64, sizeof(u8));
    }
    for (k = 0; k < 4; k++) {
        map_u16_32(b16, a16 + k);
        mixbuf(b16, 32, sizeof(u16));
    }
    for (k = 0; k < 16; k++) {
        map_u64_8(b64, a64 + k);
        mixbuf(b64, 8, sizeof(u64));
    }
    map_i32_32(bi32, ai32);
    mixbuf(bi32, 32, sizeof(int));
    map_f32_32(bf32, af32);
    mixbuf(bf32, 32, sizeof(float));
    map_f64_8(bf64, af64);
    mixbuf(bf64, 8, sizeof(double));
    map2_u32_32(c32, a32, b32);
    mixbuf(c32, 32, sizeof(u32));

    /* 2 — non-multiples (mirror runs). */
    map_u32_33(c32, a32);
    mixbuf(c32, 33, sizeof(u32));
    map_u8_67(b8, a8);
    mixbuf(b8, 67, sizeof(u8));
    map_u16_35(b16, a16);
    mixbuf(b16, 35, sizeof(u16));
    map_u64_9(b64, a64);
    mixbuf(b64, 9, sizeof(u64));
    map_f32_35(bf32, af32);
    mixbuf(bf32, 35, sizeof(float));

    /* 3 — escaping counters: the value must be the ELEMENT trip count. */
    mix((u64)map_esc_u32(c32, a32));
    mixbuf(c32, 32, sizeof(u32));
    mix((u64)map_esc_u8(b8, a8));
    mixbuf(b8, 64, sizeof(u8));
    mix((u64)map_esc_u64(b64, a64));
    mixbuf(b64, 8, sizeof(u64));
    mix((u64)map_esc_i64(bf64, af64));
    mixbuf(bf64, 8, sizeof(double));

    /* 4 — may-alias: in place, and at both dependence distances. */
    map_alias_u32(c32, c32);          /* distance 0 (in place) */
    mixbuf(c32, 32, sizeof(u32));
    map_alias_u32(c32, c32 + 1);      /* source one element AHEAD */
    mixbuf(c32, 32, sizeof(u32));
    map_alias_u32(c32 + 1, c32);      /* destination one element AHEAD */
    mixbuf(c32, 32, sizeof(u32));
    map_alias_fwd(c32, c32);          /* d[i] = s[i+1] */
    mixbuf(c32, 32, sizeof(u32));
    map_alias_bwd(c32, c32);          /* d[i+1] = s[i] */
    mixbuf(c32, 32, sizeof(u32));

    /* 5 — dynamic trip counts, including the empty and single-step cases. */
    {
        int ns[6];
        int j;
        ns[0] = 0;
        ns[1] = 1;
        ns[2] = 7;
        ns[3] = 8;
        ns[4] = 32;
        ns[5] = 33;
        for (j = 0; j < 6; j++) {
            map_dyn_u32(c32, a32 + j, ns[j]);
            mixbuf(c32, 32, sizeof(u32));
        }
    }

    /* 6 — nested: four rows of a 32-element inner map. */
    map_nested_u32(c32, a32, 4);
    mixbuf(c32, 4 * 32, sizeof(u32));

    printf("%016llx\n", (unsigned long long)state);
    return 0;
}
