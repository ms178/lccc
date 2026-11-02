/*
 * BB-SLP v3 contracts: constant-lane materialization end to end.
 *
 * Every shape here was a real defect or a hard miss in the S29 revision:
 *
 *  - v3_gather_fp_const: an FP CONSTANT lane reaching the 2-lane gather
 *    (VecPackF64x2) materialized as +0.0 — `operand_to_reg`'s catch-all
 *    zeroed the register instead of loading the bit pattern. The lane
 *    must arrive bit-exact (movabsq of the f64 bits, runtime-checked).
 *  - v3_zstore_f64 / v3_zstore_i32 / v3_zstore_i64: all-same CONSTANT
 *    store seeds (q[0..3] = 0) never vectorized at all: the root Splat
 *    pack has no BinOp consumer, so the schedule fixpoint left it
 *    unresolved and the plan bailed. They must now emit the zero vector
 *    (vxorpd/vpxor family) + ONE vector store.
 *  - v3_ones_i32 / v3_ones_i64: all-ones constant splats must use the
 *    self-compare materialization (pcmpeqd/vpcmpeqd reg,reg) — one
 *    instruction — instead of the staged mov/movd/shuffle chains.
 *  - v3_c15_f64: non-zero FP constant splats are audited-sound through
 *    emit_fp_operand_to_xmm (const-pool movsd + broadcast); the old pass
 *    rejected them on a GPR-staging assumption that only ever applied
 *    to the integer families.
 *  - v3_nzstore_f64: -0.0 has its sign bit set; `is_all_zero_bits`
 *    (not `== 0.0`) must keep it OFF the zero-vector path. Runtime
 *    signbit check is the contract (either a bit-exact pair-merge or a
 *    bit-exact broadcast is acceptable; a zero-splat is NOT).
 */
#include <stdio.h>
#include <math.h>
#include <string.h>

#if defined(NO_MAIN)
/* codegen-only translation unit: same bodies, no main. */
#else
static int fails = 0;
#define CHECK(cond, msg) \
    do { if (!(cond)) { printf("FAIL: %s\n", msg); fails++; } } while (0)
#endif

void v3_gather_fp_const(double *restrict q, const double *restrict m,
                        const double *restrict n, double x) {
    double a = m[0] + n[0];
    double b = m[1] + n[1];
    q[0] = a + 1.5; /* gather lane 0: Const(1.5) */
    q[1] = b + x;   /* gather lane 1: Value(x)   */
}

void v3_zstore_f64(double *q) {
    q[0] = 0.0; q[1] = 0.0; q[2] = 0.0; q[3] = 0.0;
}

void v3_zstore_i32(int *q) {
    q[0] = 0; q[1] = 0; q[2] = 0; q[3] = 0;
}

void v3_zstore_i64(long long *q) {
    q[0] = 0; q[1] = 0; q[2] = 0; q[3] = 0;
}

void v3_ones_i32(int *q) {
    q[0] = -1; q[1] = -1; q[2] = -1; q[3] = -1;
}

void v3_ones_i64(long long *q) {
    q[0] = -1; q[1] = -1; q[2] = -1; q[3] = -1;
}

void v3_c15_f64(double *q) {
    q[0] = 1.5; q[1] = 1.5; q[2] = 1.5; q[3] = 1.5;
}

void v3_nzstore_f64(double *q) {
    q[0] = -0.0; q[1] = -0.0;
}

/* 256-bit byte family (AVX2 I8x32): 32-byte copy and in-place add. */
void v3_bcopy32(unsigned char *restrict q, const unsigned char *restrict m) {
    q[0] = m[0]; q[1] = m[1]; q[2] = m[2]; q[3] = m[3];
    q[4] = m[4]; q[5] = m[5]; q[6] = m[6]; q[7] = m[7];
    q[8] = m[8]; q[9] = m[9]; q[10] = m[10]; q[11] = m[11];
    q[12] = m[12]; q[13] = m[13]; q[14] = m[14]; q[15] = m[15];
    q[16] = m[16]; q[17] = m[17]; q[18] = m[18]; q[19] = m[19];
    q[20] = m[20]; q[21] = m[21]; q[22] = m[22]; q[23] = m[23];
    q[24] = m[24]; q[25] = m[25]; q[26] = m[26]; q[27] = m[27];
    q[28] = m[28]; q[29] = m[29]; q[30] = m[30]; q[31] = m[31];
}

void v3_badd32(unsigned char *restrict q, const unsigned char *restrict m) {
    q[0] += m[0]; q[1] += m[1]; q[2] += m[2]; q[3] += m[3];
    q[4] += m[4]; q[5] += m[5]; q[6] += m[6]; q[7] += m[7];
    q[8] += m[8]; q[9] += m[9]; q[10] += m[10]; q[11] += m[11];
    q[12] += m[12]; q[13] += m[13]; q[14] += m[14]; q[15] += m[15];
    q[16] += m[16]; q[17] += m[17]; q[18] += m[18]; q[19] += m[19];
    q[20] += m[20]; q[21] += m[21]; q[22] += m[22]; q[23] += m[23];
    q[24] += m[24]; q[25] += m[25]; q[26] += m[26]; q[27] += m[27];
    q[28] += m[28]; q[29] += m[29]; q[30] += m[30]; q[31] += m[31];
}

/* 256-bit halfword family (AVX2 I16x16): 16-lane copy and the nested
 * promoted tree q[i]*m[i]*k (intermediates stay I32 until the store's
 * trunc — the multi-level demotion contract). */
void v3_wcopy16(short *restrict q, const short *restrict m) {
    q[0] = m[0]; q[1] = m[1]; q[2] = m[2]; q[3] = m[3];
    q[4] = m[4]; q[5] = m[5]; q[6] = m[6]; q[7] = m[7];
    q[8] = m[8]; q[9] = m[9]; q[10] = m[10]; q[11] = m[11];
    q[12] = m[12]; q[13] = m[13]; q[14] = m[14]; q[15] = m[15];
}

void v3_wmul16(short *restrict q, const short *restrict m, short k) {
    q[0] = q[0] * m[0] * k; q[1] = q[1] * m[1] * k;
    q[2] = q[2] * m[2] * k; q[3] = q[3] * m[3] * k;
    q[4] = q[4] * m[4] * k; q[5] = q[5] * m[5] * k;
    q[6] = q[6] * m[6] * k; q[7] = q[7] * m[7] * k;
    q[8] = q[8] * m[8] * k; q[9] = q[9] * m[9] * k;
    q[10] = q[10] * m[10] * k; q[11] = q[11] * m[11] * k;
    q[12] = q[12] * m[12] * k; q[13] = q[13] * m[13] * k;
    q[14] = q[14] * m[14] * k; q[15] = q[15] * m[15] * k;
}

/* Externally-used lanes on the 256-bit families: the seed must still
 * fire, with lane extracts servicing the surviving uses. The F64 shape
 * also pins the 8-byte extract-dest store (a 16-byte movdqu into the
 * F64 slot overflows into the neighbour — the seed vector's own spill
 * — and dq[0] observed lane 3's value before the fix). */
long long v3_liveout_i64(const long long *restrict m, long long *restrict q) {
    long long a = m[0], b = m[1], c = m[2], d = m[3];
    q[0] = a; q[1] = b; q[2] = c; q[3] = d;
    return b + d; /* external uses of lanes 1 and 3 */
}

double v3_liveout_f64(const double *restrict m, double *restrict q) {
    double a = m[0], b = m[1], c = m[2], d = m[3];
    q[0] = a; q[1] = b; q[2] = c; q[3] = d;
    return c; /* external use of lane 2 */
}

/* Per-lane commutative reordering: mixed operand spellings (a[i]*k vs
 * k*a[i]) leave no packable whole side — the identity/swap attempts
 * degrade to gathers and the cost model rejects the seed. The flip
 * aligns load-kind operands on one side and the splat on the other. */
void v3_mixed_sides(int *restrict q, const int *restrict a, const int *restrict b, int k) {
    q[0] = a[0] * k + b[0];
    q[1] = k * a[1] + b[1];
    q[2] = a[2] * k + b[2];
    q[3] = k * a[3] + b[3];
}

/* BB-SLP seeds INSIDE a loop that writes the loaded memory: the seed's
 * VecLoads feed stores and the same loop rewrites the loaded locations
 * (the nbody advance shape — global streams, so the loop-carried
 * forwarder leaves the load/store form intact). LICM's generic
 * pure-intrinsic hoist used to treat the VecLoad families as
 * operand-determined (is_pure is side-effect-free, not memory-read-
 * free) and lifted the loads+compute out of the loop — the positions
 * then replayed the step-0 velocities for every iteration. The runtime
 * contract pins the fix. */
static double g_vel[2];
static double g_pos[2];

void v3_loop_licm(int n, int m) {
    for (int k = 0; k < n; k++) {
        for (int i = 0; i < m; i++) {
            /* velocity loop (runtime bound so it stays a loop and the
             * store->load forwarder cannot cross it — exactly nbody's
             * j-loop): scalar writes to the stream the position block
             * then reads. */
            g_vel[i] = g_vel[i] * 1.0001 + 0.0001;
        }
        /* position block: the 2xF64 seed (Mul(MemLoad, splat)). Its
         * VecLoads live inside the STEP loop — hoisting them replays
         * step-0 velocities forever. */
        g_pos[0] = 0.01 * g_vel[0];
        g_pos[1] = 0.01 * g_vel[1];
    }
}

/* Bit-exactness of the splat detector: {-0.0, +0.0, ...} must NOT
 * collapse into a zero vector (the sign bits are data — blendv masks),
 * and an all-(-0.0) run must broadcast bit-exactly. */
void v3_negzero_mix(double *restrict q) {
    q[0] = -0.0; q[1] = 0.0; q[2] = -0.0; q[3] = 0.0;
}

void v3_negzero_all(double *restrict q) {
    q[0] = -0.0; q[1] = -0.0; q[2] = -0.0; q[3] = -0.0;
}

#if !defined(NO_MAIN)
int main(void) {
    double q[8];
    long long iq8[8];
    int iq[8];
    double m[2] = {1.0, 2.0}, n[2] = {10.0, 20.0};

    v3_gather_fp_const(q, m, n, 3.0);
    CHECK(q[0] == 12.5, "gather_fp_const lane 0 (const lane must be 1.5, not 0.0)");
    CHECK(q[1] == 25.0, "gather_fp_const lane 1");

    v3_zstore_f64(q);
    CHECK(q[0] == 0.0 && q[1] == 0.0 && q[2] == 0.0 && q[3] == 0.0,
          "zstore_f64 values");

    v3_zstore_i32(iq);
    CHECK(iq[0] == 0 && iq[1] == 0 && iq[2] == 0 && iq[3] == 0, "zstore_i32 values");

    v3_zstore_i64(iq8);
    CHECK(iq8[0] == 0 && iq8[1] == 0 && iq8[2] == 0 && iq8[3] == 0,
          "zstore_i64 values");

    v3_ones_i32(iq);
    CHECK(iq[0] == -1 && iq[1] == -1 && iq[2] == -1 && iq[3] == -1,
          "ones_i32 values");

    v3_ones_i64(iq8);
    CHECK(iq8[0] == -1 && iq8[1] == -1 && iq8[2] == -1 && iq8[3] == -1,
          "ones_i64 values");

    v3_c15_f64(q);
    CHECK(q[0] == 1.5 && q[1] == 1.5 && q[2] == 1.5 && q[3] == 1.5,
          "c15_f64 values");

    v3_nzstore_f64(q);
    CHECK(signbit(q[0]) && signbit(q[1]),
          "nzstore_f64 sign bits (-0.0 must not become +0.0)");

    {
        unsigned char m[32], qq[32];
        short wm[16], wq[16];
        for (int i = 0; i < 32; i++) {
            m[i] = (unsigned char)(i * 3 + 1);
            qq[i] = (unsigned char)(100 + i);
        }
        for (int i = 0; i < 16; i++) {
            wm[i] = (short)(i + 100);
            wq[i] = (short)(i - 8);
        }
        v3_bcopy32(qq, m);
        CHECK(memcmp(qq, m, 32) == 0, "bcopy32 values");
        for (int i = 0; i < 32; i++)
            qq[i] = (unsigned char)(100 + i);
        v3_badd32(qq, m);
        {
            int bad = 0;
            for (int i = 0; i < 32; i++)
                if (qq[i] != (unsigned char)((100 + i) + (i * 3 + 1)))
                    bad++;
            CHECK(bad == 0, "badd32 wrapping byte sums");
        }
        v3_wcopy16(wq, wm);
        {
            int bad = 0;
            for (int i = 0; i < 16; i++)
                if (wq[i] != wm[i])
                    bad++;
            CHECK(bad == 0, "wcopy16 values");
        }
        for (int i = 0; i < 16; i++)
            wq[i] = (short)(i - 8);
        v3_wmul16(wq, wm, 3);
        {
            int bad = 0;
            /* The i32-computed, truncated reference — pins the mod-2^16
             * equivalence of the nested demotion. */
            for (int i = 0; i < 16; i++) {
                int ref = (int)((i - 8) * (i + 100) * 3);
                if (wq[i] != (short)ref)
                    bad++;
            }
            CHECK(bad == 0, "wmul16 nested promoted tree demotion");
        }
    }

    {
        long long m[4] = {1, 2, 3, 4}, q[4];
        double dm[4] = {1.5, 2.5, 3.5, 4.5}, dq[4];
        long long r1 = v3_liveout_i64(m, q);
        CHECK(r1 == 6, "liveout_i64 extracted lanes (1 and 3)");
        CHECK(q[0] == 1 && q[1] == 2 && q[2] == 3 && q[3] == 4,
              "liveout_i64 stored lanes");
        double r2 = v3_liveout_f64(dm, dq);
        CHECK(r2 == 3.5, "liveout_f64 extracted lane 2");
        CHECK(dq[0] == 1.5 && dq[1] == 2.5 && dq[2] == 3.5 && dq[3] == 4.5,
              "liveout_f64 stored lanes (8-byte extract store, no slot overflow)");
    }

    {
        int a[4] = {1, 2, 3, 4}, b[4] = {10, 20, 30, 40}, q[4];
        v3_mixed_sides(q, a, b, 5);
        CHECK(q[0] == 15 && q[1] == 30 && q[2] == 45 && q[3] == 60,
              "mixed_sides per-lane flip values");
    }

    {
        /* Reference: the same evolution in long double (no SLP family).
         * The vectorized loop must match within FP contraction tolerance —
         * NOT replay the step-0 velocities. */
        long double rv[2] = {1.0L, -2.0L}, rp[2] = {0.0L, 0.0L};
        g_vel[0] = 1.0;
        g_vel[1] = -2.0;
        v3_loop_licm(20000, 2);
        for (int k = 0; k < 20000; k++) {
            rv[0] = rv[0] * 1.0001L + 0.0001L;
            rv[1] = rv[1] * 1.0001L + 0.0001L;
            rp[0] = 0.01L * rv[0];
            rp[1] = 0.01L * rv[1];
        }
        double tol = 1e-6;
        CHECK(fabs(g_pos[0] - (double)rp[0]) < tol && fabs(g_pos[1] - (double)rp[1]) < tol,
              "loop_licm: in-loop VecLoads must not be hoisted (nbody class)");
        CHECK(fabs(g_vel[0] - (double)rv[0]) < tol && fabs(g_vel[1] - (double)rv[1]) < tol,
              "loop_licm: velocity evolution");
    }

    {
        double q[4];
        v3_negzero_mix(q);
        CHECK(signbit(q[0]) && !signbit(q[1]) && signbit(q[2]) && !signbit(q[3]),
              "negzero_mix: mixed-sign zero lanes keep their sign bits");
        v3_negzero_all(q);
        CHECK(signbit(q[0]) && signbit(q[1]) && signbit(q[2]) && signbit(q[3]),
              "negzero_all: the -0.0 splat broadcasts bit-exactly");
    }

    printf("bb_slp_v3: all pass (%d fails)\n", fails);
    return fails != 0;
}
#endif
