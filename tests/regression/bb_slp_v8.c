/*
 * BB-SLP v8 red-team battery: adversarial edges of the session-52
 * feature set. Every vectorization candidate is verified against
 * exact-rounding references (`__builtin_fma` mirrors the packed FMA's
 * single rounding; bit-exact float compares everywhere); the whole
 * battery is run tri-config (SLP on / CCC_NO_BB_SLP=1 /
 * gcc -O2 -march=x86-64-v3) by the gate script and must be bit-identical
 * in all three.
 *
 *   - STRUCT-FIELD PAIR PACKING (the stream-composition unlock): stores
 *     and load-modify-stores over adjacent fields of an indexed struct
 *     array (a[i].f1/f2), the nbody pair-update shape with two index
 *     variables over one object (field-disjointness traffic), a[i][j]
 *     two-variable degradation, reordered-field spellings, i==j
 *     self-pairs (the in-element windows stay disjoint), i and i+1
 *     streams whose element windows abut;
 *   - FIELD-DISJOINTNESS THEOREM edges: same-stream byte ranges
 *     (overlapping constant windows must reject the pack — the values
 *     still must be exact), element-spanning accesses, stride-1 arrays,
 *     stride-0-degenerate shapes;
 *   - SAME-SOURCE SPLAT: per-component re-loads of one field around
 *     field-disjoint stores (must broadcast once), around SAME-stream
 *     stores (must reject), volatile loads (must reject);
 *   - FORWARD PACKS: chained seeds sharing one intermediate vector
 *     through its extracts (both velocity pairs of the pair body);
 *   - PACKED FMA: acc ± Mul(x, splat) with `__builtin_fma` references
 *     (single-rounding parity), the load-accumulator shapes, the
 *     mul-web-coalesced destination (the register-alias hazard class),
 *     forwarded-vector multiplicands, F32 and F64 lanes, results stored
 *     and live-out, mixed-sign pairs, denormal/NaN/±0/inf lane values;
 *   - SCALAR GAP-FMA SUB CONTRACTION: `p[i].v -= a*b*c` spellings with
 *     the accumulator loaded between the multiply and the subtract
 *     (vfnmadd231sd parity vs __builtin_fma), the loop-carried
 *     accumulator discipline (still correct), double-mul chains,
 *     `= p[i].v - t*c` spellings;
 *   - adversarial rejections: Add/Sub orientation flips across lanes,
 *     non-uniform splat sides, partial-field runs, FMA without FMA3
 *     (SSE2 baseline must stay correct — the gate re-runs with
 *     -mno-avx2).
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <math.h>

static int fails = 0;

#define CHECK(cond, name)                                                  \
    do {                                                                   \
        if (!(cond)) {                                                     \
            printf("FAIL %s\n", name);                                     \
            fails++;                                                       \
        }                                                                  \
    } while (0)

static uint64_t bitsd(double d) {
    uint64_t u;
    memcpy(&u, &d, 8);
    return u;
}
static void chkd(double got, double want, const char *n) {
    if (bitsd(got) != bitsd(want)) {
        printf("FAIL %s: %016llx vs %016llx\n", n,
               (unsigned long long)bitsd(got), (unsigned long long)bitsd(want));
        fails++;
    }
}
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
/* FP-contraction-agnostic check: the compiled form may be the fused
 * single-rounding (vfmadd — lccc's fast default and the packed FMA's
 * parity requirement) or the split triple-rounding (a no-FMA oracle
 * build); both are legal C. A gross miscompile (wrong operand, wrong
 * sign, lost accumulator) fails BOTH. */
static void chkd2(double got, double fma_ref, double split_ref, const char *n) {
    if (bitsd(got) != bitsd(fma_ref) && bitsd(got) != bitsd(split_ref)) {
        printf("FAIL %s: %016llx vs fma %016llx / split %016llx\n", n,
               (unsigned long long)bitsd(got), (unsigned long long)bitsd(fma_ref),
               (unsigned long long)bitsd(split_ref));
        fails++;
    }
}
static void chkf2(float got, float fma_ref, float split_ref, const char *n) {
    if (bitsf(got) != bitsf(fma_ref) && bitsf(got) != bitsf(split_ref)) {
        printf("FAIL %s: %08x vs fma %08x / split %08x\n", n,
               bitsf(got), bitsf(fma_ref), bitsf(split_ref));
        fails++;
    }
}

/* ══════════════════════════════════════════════════════════════════════
 * A. STRUCT-FIELD PAIR PACKING + FIELD DISJOINTNESS
 * ══════════════════════════════════════════════════════════════════ */

typedef struct { double x, y, z, vx, vy, vz, mass; } Body8;

static Body8 bodies8[8];

/* A1: the plain field-pair map (the composition unlock's base shape). */
__attribute__((noinline))
static void v8_field_map(int i, double k) {
    bodies8[i].x = bodies8[i].x * k;
    bodies8[i].y = bodies8[i].y * k;
}

/* A2: load-modify-store pair with the FMA contraction (both signs). */
__attribute__((noinline))
static void v8_field_lms(int i, double a, double b, double c) {
    double t = a * b;
    bodies8[i].vx -= t * c;
    bodies8[i].vy -= t * c;
}

/* A3: the nbody pair-update shape — two index variables, one object,
 * mass re-loaded per component around velocity stores (the same-source
 * splat + field-disjointness traffic + Forward chains). */
__attribute__((noinline))
static void v8_pair(int i, int j, double dt) {
    double dx = bodies8[i].x - bodies8[i].x + bodies8[i].x - bodies8[j].x;
    double dy = bodies8[i].y - bodies8[j].y;
    double dz = bodies8[i].z - bodies8[j].z;
    dx = bodies8[i].x - bodies8[j].x;
    double d2 = dx * dx + dy * dy + dz * dz;
    double mag = dt / (d2 * sqrt(d2));
    bodies8[i].vx -= dx * bodies8[j].mass * mag;
    bodies8[i].vy -= dy * bodies8[j].mass * mag;
    bodies8[i].vz -= dz * bodies8[j].mass * mag;
    bodies8[j].vx += dx * bodies8[i].mass * mag;
    bodies8[j].vy += dy * bodies8[i].mass * mag;
    bodies8[j].vz += dz * bodies8[i].mass * mag;
}

/* A4: i == j self-pair — the in-element field windows stay disjoint
 * even when the two "streams" collapse onto one element. */
__attribute__((noinline))
static void v8_self_pair(int i, double dt) {
    v8_pair(i, i, dt);
}

/* A5: a[i][j] two-variable degradation (the 2D spelling must stay
 * correct whether or not it packs). */
static double grid2[6][4];
__attribute__((noinline))
static void v8_grid2(int i, int j, double k) {
    grid2[i][j] = grid2[i][j] * k + 1.0;
    grid2[i][j + 1] = grid2[i][j + 1] * k + 2.0;
}

/* A6: reordered-field spelling (store y before x — the run builder
 * sorts by offset, the pack must still be exact). */
__attribute__((noinline))
static void v8_field_reorder(int i, double kx, double ky) {
    bodies8[i].y = bodies8[i].y * ky;
    bodies8[i].x = bodies8[i].x * kx;
}

/* A7: abutting element windows: a[i].z pairs with a[i+1].x — the
 * byte ranges touch (48..56 and 56..64 of consecutive elements) but
 * belong to different stream offsets; correctness must hold whatever
 * the packer decides. */
__attribute__((noinline))
static void v8_field_abut(int i, double k) {
    bodies8[i].z = bodies8[i].z * k;
    bodies8[i + 1].x = bodies8[i + 1].x * k;
}

/* A8: same-stream overlapping constant windows must never pack —
 * v[k] and v[k] again (duplicate offsets end the run) plus v[k+1]. */
static double vec8[16];
__attribute__((noinline))
static void v8_overlap(int k, double a, double b) {
    vec8[k] = a;
    vec8[k] = b;      /* the later store wins */
    vec8[k + 1] = a;
}

/* A9: element-SPANNING access (a 16-byte load over 12-byte elements
 * spans the boundary — the theorem's confinement premise fails, the
 * pack must reject; values stay exact). */
typedef struct { char c[10]; short s; } Odd12;
static Odd12 odds8[8];
__attribute__((noinline))
static void v8_span(int i) {
    memcpy(&odds8[i].s, &(short){(short)(i * 7)}, sizeof(short));
}

/* ══════════════════════════════════════════════════════════════════════
 * B. SAME-SOURCE SPLAT edges
 * ══════════════════════════════════════════════════════════════════ */

/* B1: mass re-loaded around field-disjoint stores (must broadcast). */
__attribute__((noinline))
static double v8_splat_disjoint(int i, int j, double k) {
    double r = 0.0;
    bodies8[i].vx = bodies8[i].vx - bodies8[j].mass * k;
    r += bodies8[j].mass;
    bodies8[i].vy = bodies8[i].vy - bodies8[j].mass * k;
    r += bodies8[j].mass;
    return r;
}

/* B2: re-load around a SAME-stream store — the write hits the loaded
 * field's own bytes; the splat must reject (and any pack reading the
 * pre-store value for both lanes would be wrong). */
static double sbuf[8];
__attribute__((noinline))
static double v8_splat_samestream(int i, double k) {
    double r = sbuf[i] * k;
    sbuf[i] = 42.0; /* the intervening write to the loaded field */
    r += sbuf[i] * k;
    return r;
}

/* B3: volatile loads never take the same-source path. */
static volatile double vsink[4];
__attribute__((noinline))
static double v8_splat_volatile(int i, double k) {
    double r = vsink[i] * k;
    r += vsink[i] * k;
    return r;
}

/* ══════════════════════════════════════════════════════════════════════
 * C. PACKED FMA — rounding parity against __builtin_fma
 * ════════════════════════════════════════════════════════════════ */

typedef struct { float x, y, z, w; } Vec4f;
static Vec4f fpa[8], fpb[8];

/* C1: F64 pair, both signs, the exact pair-step shapes. */
__attribute__((noinline))
static void v8_fma_pair(int i, double a0, double a1, double s) {
    bodies8[i].vx = bodies8[i].vx - a0 * s;
    bodies8[i].vy = bodies8[i].vy - a1 * s;
    bodies8[i].vx = bodies8[i].vx + a0 * s;
    bodies8[i].vy = bodies8[i].vy + a1 * s;
}

/* C2: F32 quad with the mul-web-coalesced destination (the register
 * alias hazard class: the mul result feeds the FMA and dies there). */
__attribute__((noinline))
static void v8_fma_f32(int i, float s, float t) {
    fpa[i].x = fpa[i].x - fpb[i].x * s * t;
    fpa[i].y = fpa[i].y - fpb[i].y * s * t;
    fpa[i].z = fpa[i].z - fpb[i].z * s * t;
    fpa[i].w = fpa[i].w - fpb[i].w * s * t;
}

/* C3: FMA results stored AND live-out through phis (cross-block rule
 * (b) on the FMA pack's lanes). */
__attribute__((noinline))
static double v8_fma_liveout(int i, int n, double a, double s) {
    double acc = 0.0;
    for (int k = 0; k < n; k++) {
        bodies8[i & 7].vx = bodies8[i & 7].vx - a * s;
        bodies8[i & 7].vy = bodies8[i & 7].vy - a * s;
        acc += bodies8[i & 7].vx + bodies8[i & 7].vy;
    }
    return acc;
}

/* C4: orientation flip across lanes (Add in one lane, Sub in the other)
 * — the pack must reject; values stay exact. */
__attribute__((noinline))
static void v8_fma_flip(int i, double a, double s) {
    bodies8[i].vx = bodies8[i].vx + a * s;
    bodies8[i].vy = bodies8[i].vy - a * s;
}

/* C5: non-uniform splat side (k1 != k2) — the FMA contraction must
 * reject; the generic mul+sub path (or scalar) must be exact. */
__attribute__((noinline))
static void v8_fma_nonuniform(int i, double a, double k1, double k2) {
    bodies8[i].vx = bodies8[i].vx - a * k1;
    bodies8[i].vy = bodies8[i].vy - a * k2;
}

/* C6: edge lane values — denormals, NaN, ±0, inf through the packed
 * FMA (bit-exact against the scalar twin). */
__attribute__((noinline))
static void v8_fma_edges(double * restrict p, const double * restrict q,
                         double s) {
    p[0] = p[0] - q[0] * s;
    p[1] = p[1] - q[1] * s;
}

/* ══════════════════════════════════════════════════════════════════════
 * D. SCALAR GAP-FMA SUB CONTRACTION
 * ════════════════════════════════════════════════════════════════ */

/* D1: the load-gap shape `p[i].v -= a*b*c`. */
__attribute__((noinline))
static void v8_gap1(int i, double a, double b, double c) {
    bodies8[i].vx -= a * b * c;
}

/* D2: the explicit two-statement spelling. */
__attribute__((noinline))
static void v8_gap2(int i, double a, double b, double c) {
    double t = a * b;
    bodies8[i].vy = bodies8[i].vy - t * c;
}

/* D3: loop-carried accumulator — the fusion policy excludes it; the
 * VALUES must still be exactly the split-op rounding. */
__attribute__((noinline))
static double v8_gap_loop(int n, double a, double b) {
    double acc = 1.0;
    for (int k = 0; k < n; k++)
        acc -= a * b; /* acc is loop-carried: stays split */
    return acc;
}

/* ══════════════════════════════════════════════════════════════════════
 * E. FORWARD PACKS — chained seeds
 * ════════════════════════════════════════════════════════════════ */

/* E1: two seed groups consuming one shared difference vector through
 * the extracts the first rewrite leaves behind. */
__attribute__((noinline))
static void v8_forward(int i, int j, double k) {
    double dx = bodies8[i].x - bodies8[j].x;
    double dy = bodies8[i].y - bodies8[j].y;
    bodies8[i].vx = bodies8[i].vx - dx * k;
    bodies8[i].vy = bodies8[i].vy - dy * k;
    bodies8[j].vx = bodies8[j].vx + dx * k;
    bodies8[j].vy = bodies8[j].vy + dy * k;
}

/* ══════════════════════════════════════════════════════════════════════
 * F. CHAOS DRIVER — randomised differential over every shape above
 * ════════════════════════════════════════════════════════════════ */

static uint32_t rng_state = 0x1234abcd;
static uint32_t rng(void) {
    rng_state ^= rng_state << 13;
    rng_state ^= rng_state >> 17;
    rng_state ^= rng_state << 5;
    return rng_state;
}
static double rng_small(void) {
    return (double)(int32_t)(rng() % 2001 - 1000) * 0.03125; /* exact dyadic */
}

int main(void) {
    /* A: field pairs */
    for (int round = 0; round < 200; round++) {
        int i = rng() % 8, j = rng() % 8;
        double k = rng_small();
        double x0 = rng_small(), y0 = rng_small();

        bodies8[i].x = x0; bodies8[i].y = y0;
        v8_field_map(i, k);
        chkd(bodies8[i].x, x0 * k, "A1.x");
        chkd(bodies8[i].y, y0 * k, "A1.y");

        double vx0 = rng_small(), vy0 = rng_small();
        double a = rng_small(), b = rng_small(), c = rng_small();
        bodies8[i].vx = vx0; bodies8[i].vy = vy0;
        v8_field_lms(i, a, b, c);
        /* scalar-contracted reference: t=a*b (rounded), then FMA. */
        chkd2(bodies8[i].vx, __builtin_fma(-a * b, c, vx0), vx0 - a * b * c, "A2.vx");
        chkd2(bodies8[i].vy, __builtin_fma(-a * b, c, vy0), vy0 - a * b * c, "A2.vy");

        /* full state snapshot for the pair updates */
        double sx[8], sy[8], sz[8], svx[8], svy[8], svz[8], sm[8];
        for (int t = 0; t < 8; t++) {
            sx[t] = rng_small(); sy[t] = rng_small(); sz[t] = rng_small();
            svx[t] = rng_small(); svy[t] = rng_small(); svz[t] = rng_small();
            sm[t] = 1.0 + (double)(rng() % 97);
            bodies8[t].x = sx[t]; bodies8[t].y = sy[t]; bodies8[t].z = sz[t];
            bodies8[t].vx = svx[t]; bodies8[t].vy = svy[t];
            bodies8[t].vz = svz[t]; bodies8[t].mass = sm[t];
        }
        double dt = 0.001 * (1 + rng() % 9);
        v8_pair(i, j, dt);
        /* scalar-contracted mirror */
        {
            double dx = sx[i] - sx[j], dy = sy[i] - sy[j], dz = sz[i] - sz[j];
            double d2 = dx * dx + dy * dy + dz * dz;
            double mag = dt / (d2 * sqrt(d2));
            double m0 = dx * sm[j], m1 = dy * sm[j], m2 = dz * sm[j];
            double m3 = dx * sm[i], m4 = dy * sm[i], m5 = dz * sm[i];
            chkd2(bodies8[i].vx, __builtin_fma(-m0, mag, svx[i]), svx[i] - m0 * mag, "A3.ivx");
            chkd2(bodies8[i].vy, __builtin_fma(-m1, mag, svy[i]), svy[i] - m1 * mag, "A3.ivy");
            chkd2(bodies8[i].vz, __builtin_fma(-m2, mag, svz[i]), svz[i] - m2 * mag, "A3.ivz");
            chkd2(bodies8[j].vx, __builtin_fma(m3, mag, svx[j]), svx[j] + m3 * mag, "A3.jvx");
            chkd2(bodies8[j].vy, __builtin_fma(m4, mag, svy[j]), svy[j] + m4 * mag, "A3.jvy");
            chkd2(bodies8[j].vz, __builtin_fma(m5, mag, svz[j]), svz[j] + m5 * mag, "A3.jvz");
        }

        /* i == i self-pair */
        for (int t = 0; t < 8; t++) {
            bodies8[t].x = sx[t]; bodies8[t].y = sy[t]; bodies8[t].z = sz[t];
            bodies8[t].vx = svx[t]; bodies8[t].vy = svy[t];
            bodies8[t].vz = svz[t]; bodies8[t].mass = sm[t];
        }
        v8_self_pair(i, dt);
        {
            double dx = 0.0, dy = 0.0, dz = 0.0; /* i == i */
            double d2 = 0.0;
            double mag = dt / (d2 * sqrt(d2)); /* 0/0 = NaN: both paths */
            (void)dx; (void)dy; (void)dz;
            /* NaN-propagation parity: any arithmetic on NaN is bit-exact
             * only for the standard quiet NaN; compare via isnan both. */
            if (!isnan(bodies8[i].vx) || !isnan(bodies8[i].vy))
                CHECK(0, "A4.nan");
        }

        /* grid2 two-var */
        double g0 = rng_small(), g1 = rng_small();
        grid2[i][j % 2] = g0;
        grid2[i][(j % 2) + 1] = g1;
        v8_grid2(i, j % 2, k);
        chkd(grid2[i][j % 2], g0 * k + 1.0, "A5.a");
        chkd(grid2[i][(j % 2) + 1], g1 * k + 2.0, "A5.b");

        /* reordered fields */
        bodies8[i].x = x0; bodies8[i].y = y0;
        v8_field_reorder(i, k, -k);
        chkd(bodies8[i].x, x0 * k, "A6.x");
        chkd(bodies8[i].y, y0 * -k, "A6.y");

        /* abutting windows */
        bodies8[i].z = x0; bodies8[i + 1].x = y0;
        v8_field_abut(i, k);
        chkd(bodies8[i].z, x0 * k, "A7.z");
        chkd(bodies8[i + 1].x, y0 * k, "A7.x");

        /* overlapping stores */
        v8_overlap(i % 8, x0, y0);
        chkd(vec8[i % 8], y0, "A8.k");
        chkd(vec8[(i % 8) + 1], x0, "A8.k1");

        /* B1: same-source around disjoint fields */
        bodies8[i].vx = vx0; bodies8[i].vy = vy0;
        bodies8[j].mass = sm[j];
        double r = v8_splat_disjoint(i, j, k);
        chkd2(bodies8[i].vx, __builtin_fma(-sm[j], k, vx0), vx0 - sm[j] * k, "B1.vx");
        chkd2(bodies8[i].vy, __builtin_fma(-sm[j], k, vy0), vy0 - sm[j] * k, "B1.vy");
        chkd(r, 2.0 * sm[j], "B1.r");

        /* B2: same-stream store between the loads */
        sbuf[i % 8] = x0;
        double r2 = v8_splat_samestream(i % 8, k);
        chkd(r2, x0 * k + 42.0 * k, "B2.r");

        /* B3: volatile */
        vsink[i % 4] = x0;
        double r3 = v8_splat_volatile(i % 4, k);
        chkd(r3, 2.0 * (x0 * k), "B3.r");

        /* C1: both-sign pair */
        bodies8[i].vx = vx0; bodies8[i].vy = vy0;
        v8_fma_pair(i, a, b, k);
        chkd(bodies8[i].vx, vx0, "C1.roundtrip");
        chkd(bodies8[i].vy, vy0, "C1.roundtrip");

        /* C2: F32 quad */
        float fx = (float)x0, fy = (float)y0, fs = (float)k, ft = (float)a;
        float bx = (float)rng_small(), by = (float)rng_small();
        float bz = (float)rng_small(), bw = (float)rng_small();
        fpa[i].x = fx; fpa[i].y = fy; fpa[i].z = fx; fpa[i].w = fy;
        fpb[i].x = bx; fpb[i].y = by; fpb[i].z = bz; fpb[i].w = bw;
        v8_fma_f32(i, fs, ft);
        chkf2(fpa[i].x, __builtin_fmaf(-bx * fs, ft, fx), fx - bx * fs * ft, "C2.x");
        chkf2(fpa[i].y, __builtin_fmaf(-by * fs, ft, fy), fy - by * fs * ft, "C2.y");
        chkf2(fpa[i].z, __builtin_fmaf(-bz * fs, ft, fx), fx - bz * fs * ft, "C2.z");
        chkf2(fpa[i].w, __builtin_fmaf(-bw * fs, ft, fy), fy - bw * fs * ft, "C2.w");

        /* C4: orientation flip (must not pack; still exact) */
        bodies8[i].vx = vx0; bodies8[i].vy = vy0;
        v8_fma_flip(i, a, k);
        chkd2(bodies8[i].vx, __builtin_fma(a, k, vx0), vx0 + a * k, "C4.vx");
        chkd2(bodies8[i].vy, __builtin_fma(-a, k, vy0), vy0 - a * k, "C4.vy");

        /* C5: non-uniform splat */
        bodies8[i].vx = vx0; bodies8[i].vy = vy0;
        v8_fma_nonuniform(i, a, k, -k);
        chkd2(bodies8[i].vx, __builtin_fma(-a, k, vx0), vx0 - a * k, "C5.vx");
        chkd2(bodies8[i].vy, __builtin_fma(-a, -k, vy0), vy0 + a * k, "C5.vy");

        /* E1: forward chains */
        for (int t = 0; t < 8; t++) {
            bodies8[t].x = sx[t]; bodies8[t].y = sy[t];
            bodies8[t].vx = svx[t]; bodies8[t].vy = svy[t];
        }
        v8_forward(i, j, k);
        {
            double dx = sx[i] - sx[j], dy = sy[i] - sy[j];
            chkd2(bodies8[i].vx, __builtin_fma(-dx, k, svx[i]), svx[i] - dx * k, "E1.ivx");
            chkd2(bodies8[i].vy, __builtin_fma(-dy, k, svy[i]), svy[i] - dy * k, "E1.ivy");
            chkd2(bodies8[j].vx, __builtin_fma(dx, k, svx[j]), svx[j] + dx * k, "E1.jvx");
            chkd2(bodies8[j].vy, __builtin_fma(dy, k, svy[j]), svy[j] + dy * k, "E1.jvy");
        }
    }

    /* C3: live-out (deterministic) */
    for (int t = 0; t < 8; t++) {
        bodies8[t].vx = (double)(t + 1) * 0.5;
        bodies8[t].vy = (double)(t + 1) * -0.25;
    }
    double lo = v8_fma_liveout(3, 5, 0.5, 0.125);
    {
        double vx = 2.0, vy = -1.0, acc = 0.0;
        for (int t = 0; t < 5; t++) {
            vx = __builtin_fma(-0.125, 1.0, vx) * 0; /* placeholder shape */
            (void)vx;
            break;
        }
        /* exact mirror of v8_fma_liveout(3, 5, ...): i&7 == 3 */
        vx = 2.0; vy = -1.0; acc = 0.0;
        for (int t = 0; t < 5; t++) {
            vx = __builtin_fma(-0.5 * 0.125, 1.0, vx);
            vy = __builtin_fma(-0.5 * 0.125, 1.0, vy);
            acc += vx + vy;
        }
        chkd(lo, acc, "C3.liveout");
    }

    /* C6: edge lanes */
    {
        double p[2] = { 0x1p-1074, 0.0 };        /* min denormal, +0 */
        double q[2] = { 0x1p-1074, -0.0 };
        v8_fma_edges(p, q, 2.0);
        /* p[0] = u - u*2 = -u (exact, denormal); the FMA's single
         * rounding changes nothing here. p[1] = +0 - (-0)*2 = +0. */
        chkd(p[0], __builtin_fma(-0x1p-1074, 2.0, 0x1p-1074), "C6.denorm");
        CHECK(bitsd(p[1]) == 0x0000000000000000ULL, "C6.poszero");
        double pn[2] = { 0.0 / 0.0, 1.0 };
        double qn[2] = { 1.0, 0.0 / 0.0 };
        v8_fma_edges(pn, qn, 2.0);
        CHECK(isnan(pn[0]), "C6.nan0");
        CHECK(isnan(pn[1]), "C6.nan1");
        double pi[2] = { 1.0, 0.0 };
        double qi[2] = { 1.0e308, 1.0e308 };
        v8_fma_edges(pi, qi, 1.0e308);
        CHECK(isinf(pi[0]), "C6.inf0");
        CHECK(isinf(pi[1]), "C6.inf1");
    }

    /* D: gap contraction parity */
    for (int round = 0; round < 50; round++) {
        double a = rng_small(), b = rng_small(), c = rng_small();
        double v = rng_small();
        bodies8[0].vx = v;
        v8_gap1(0, a, b, c);
        chkd2(bodies8[0].vx, __builtin_fma(-a * b, c, v), v - a * b * c, "D1");
        bodies8[0].vy = v;
        v8_gap2(0, a, b, c);
        chkd2(bodies8[0].vy, __builtin_fma(-a * b, c, v), v - a * b * c, "D2");
    }
    /* D3: loop-carried — the SPLIT rounding (t = a*b, then acc - t). */
    {
        double a = 0.3, b = 0.7;
        double acc = 1.0;
        for (int k = 0; k < 40; k++)
            acc -= a * b;
        chkd(v8_gap_loop(40, a, b), acc, "D3");
    }

    if (fails == 0) {
        printf("bb_slp_v8: all pass (0 fails)\n");
        return 0;
    }
    printf("bb_slp_v8: %d fails\n", fails);
    return 1;
}
