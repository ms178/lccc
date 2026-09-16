// BB-SLP red-team battery: the soundness holes found by the 2026-09-16
// semantic audit of the vectorizer, plus rule-(e) precision and the
// family-aware width fallback.
//
// Every function here is a CLOSED repro of a defect class:
//   rt_call    — lanes consumed by call arguments. The hand-rolled use
//                scan had no Call arm: the call referenced removed lane
//                definitions (dangling SSA → ICE/miscompile).
//   rt_phi     — lanes flowing into a successor-block phi. The external
//                scan had no Phi arm: rule (b) was vacuous for phi edges.
//   rt_asm     — lanes consumed by inline-asm inputs (no InlineAsm arm).
//   rt_atomic  — lanes consumed by an atomic-store value (no atomics arm).
//   rt_fwd     — store→load forwarding inside one seed (rule e): the
//                deferred vector load would read the pre-store value.
//   rt_xseed   — cross-seed hazard: seed B's loads must not reorder
//                across seed A's already-emitted vector store.
//   rt_precise — rule-(e) precision: same-stream stores/loads with
//                DISJOINT byte ranges must still vectorize.
//   rt_w16     — width fallback: a 20-lane u16 run must pack as I16x8
//                seeds, not be dropped for wanting a 16-lane family.
//   rt_w8      — same for a 34-lane byte run.
//
// Part of tests/regression/check_bb_slp_codegen.sh (runtime + codegen).
#include <stdio.h>
#include <stdint.h>
#include <stdatomic.h>

// ── RT1: lane values consumed by a CALL between defs and stores ────────
// Old bug: the hand-rolled use scan had no Call/CallIndirect arm — the
// call's arguments referenced removed lane definitions. With a profitable
// seed (4×I64, both operand sides MemLoad) this was a hard backend ICE
// ("cannot be materialised — refusing to fabricate a value"); the fix
// either rejects the seed (I64x4 has no lane extracts) or services the
// use via extracts (I64x2).
static long rt_call_sum4(long a, long b, long c, long d) {
    return a + b * 10 + c * 100 + d * 1000;
}
static long (*rt_call_fp)(long, long, long, long) = rt_call_sum4;
static long rt_call_g;
void rt_call(long *restrict q, const long *restrict w) {
    long a = q[0] + w[0], b = q[1] + w[1], c = q[2] + w[2], d = q[3] + w[3];
    rt_call_g = rt_call_fp(a, b, c, d);   // indirect call args use all 4 lanes
    q[0] = a; q[1] = b; q[2] = c; q[3] = d;
}

// ── RT2: store→load forwarding hazard (rule e) — MUST stay scalar ──────
void rt_fwd(long *q) {
    long t = q[0];
    q[1] = t;
    long u = q[1];   // reads t (post-store)
    q[2] = u;
}

// ── RT3: cross-seed hazard via an already-emitted vector store ─────────
// Call with aliasing arguments (rt_xseed(buf, buf)) for the hazard case;
// the second seed must not reorder its loads across the first seed's
// vector store.
void rt_xseed(long *q, long *r) {
    long c = q[0];
    long a0 = r[0] + 1, a1 = r[1] + 1;
    r[0] = a0; r[1] = a1;
    long d = q[1];
    q[0] = c + 1; q[1] = d + 1;
}

// ── RT4: rule-(e) precision — disjoint same-stream store/load ─────────
// Stores to q[0..1] precede loads of q[2..3] in one 4-lane seed; the
// byte ranges never overlap, so the seed is legal and SHOULD vectorize.
void rt_precise(long *q) {
    long a = q[0], b = q[1];
    q[0] = a + 1; q[1] = b + 1;
    long c = q[2], d = q[3];
    q[2] = c + 1; q[3] = d + 1;
}

// ── RT5: lanes flowing into a successor-block phi (rule b) ─────────────
long rt_phi(long *q, int cond, long w, long x) {
    long a = 0, b = 0;
    if (cond) {
        a = q[0] + w; b = q[1] + x;
        q[0] = a; q[1] = b;
    }
    return a + b;   // a/b are phi-merged here: cross-block lane uses
}

// ── RT5b: TRUE cross-block lane uses (v6 rule-(b) relaxation) — the
// lanes are consumed by instructions in DOMINATED successor blocks with
// no phi in between. The extracts ride with the pack in the defining
// block and dominate every use (the soundness proof), so this MUST
// vectorize; the self-check pins the values both sides compute.
long rt_xblock(long * restrict q, long * restrict r, int cond) {
    long a = q[0] + r[0], b = q[1] + r[1];
    q[2] = a; q[3] = b;
    if (cond) {
        return a * b;
    }
    return a - b;
}

// ── RT6: lanes consumed by inline-asm inputs ───────────────────────────
long rt_asm(long *q, long w, long x) {
    long a = q[0] + w, b = q[1] + x;
    __asm__ volatile("" :: "r"(a), "r"(b));
    q[0] = a; q[1] = b;
    return a + b;
}

// ── RT7: lanes consumed by an atomic-store value ───────────────────────
void rt_atomic(long *q, long w, long x, atomic_llong *p) {
    long a = q[0] + w, b = q[1] + x;
    atomic_store(p, b);
    q[0] = a; q[1] = b;
}

// ── RT8: width fallback — 20 consecutive u16 lanes (I16x8, not I16x16) ─
// `restrict` lets rule (e)'s escape prove the store/load bases disjoint.
typedef struct { uint16_t h[20]; } H20;
void rt_w16(H20 *restrict o, const H20 *restrict i) {
    o->h[0]=i->h[0]+1;  o->h[1]=i->h[1]+1;  o->h[2]=i->h[2]+1;  o->h[3]=i->h[3]+1;
    o->h[4]=i->h[4]+1;  o->h[5]=i->h[5]+1;  o->h[6]=i->h[6]+1;  o->h[7]=i->h[7]+1;
    o->h[8]=i->h[8]+1;  o->h[9]=i->h[9]+1;  o->h[10]=i->h[10]+1; o->h[11]=i->h[11]+1;
    o->h[12]=i->h[12]+1; o->h[13]=i->h[13]+1; o->h[14]=i->h[14]+1; o->h[15]=i->h[15]+1;
    o->h[16]=i->h[16]+1; o->h[17]=i->h[17]+1; o->h[18]=i->h[18]+1; o->h[19]=i->h[19]+1;
}

// ── RT8b: same shape WITHOUT restrict — MUST stay scalar (rule e): the
// interleaved store/load streams may alias at runtime, and GCC miscompiles
// exactly this shape under a 2-byte-shifted alias (verified 2026-09-16).
typedef struct { uint16_t h[8]; } H8nr;
void rt_w16_norestr(H8nr *o, const H8nr *i) {
    o->h[0]=i->h[0]+1; o->h[1]=i->h[1]+1; o->h[2]=i->h[2]+1; o->h[3]=i->h[3]+1;
    o->h[4]=i->h[4]+1; o->h[5]=i->h[5]+1; o->h[6]=i->h[6]+1; o->h[7]=i->h[7]+1;
}

// ── RT9: width fallback — 34 consecutive byte lanes (I8x16, not I8x32) ─
typedef struct { uint8_t b[34]; } B34;
void rt_w8(B34 *restrict o, const B34 *restrict i) {
    o->b[0]=i->b[0]+1;  o->b[1]=i->b[1]+1;  o->b[2]=i->b[2]+1;  o->b[3]=i->b[3]+1;
    o->b[4]=i->b[4]+1;  o->b[5]=i->b[5]+1;  o->b[6]=i->b[6]+1;  o->b[7]=i->b[7]+1;
    o->b[8]=i->b[8]+1;  o->b[9]=i->b[9]+1;  o->b[10]=i->b[10]+1; o->b[11]=i->b[11]+1;
    o->b[12]=i->b[12]+1; o->b[13]=i->b[13]+1; o->b[14]=i->b[14]+1; o->b[15]=i->b[15]+1;
    o->b[16]=i->b[16]+1; o->b[17]=i->b[17]+1; o->b[18]=i->b[18]+1; o->b[19]=i->b[19]+1;
    o->b[20]=i->b[20]+1; o->b[21]=i->b[21]+1; o->b[22]=i->b[22]+1; o->b[23]=i->b[23]+1;
    o->b[24]=i->b[24]+1; o->b[25]=i->b[25]+1; o->b[26]=i->b[26]+1; o->b[27]=i->b[27]+1;
    o->b[28]=i->b[28]+1; o->b[29]=i->b[29]+1; o->b[30]=i->b[30]+1; o->b[31]=i->b[31]+1;
    o->b[32]=i->b[32]+1; o->b[33]=i->b[33]+1;
}

int main(void) {
    int fails = 0;
    #define CHECK(cond, name) do { if (!(cond)) { printf("FAIL %s\n", name); fails++; } } while (0)

    long q1[8] = {10, 20, 30, 40, 50, 60, 70, 80};

    long cw[4] = {10, 20, 30, 40};
    rt_call(q1, cw);
    CHECK(q1[0] == 20 && q1[1] == 40 && q1[2] == 60 && q1[3] == 80, "rt_call stores");
    CHECK(rt_call_g == 20 + 40*10 + 60*100 + 80*1000, "rt_call arg values");

    long q2[8] = {5, 6, 7, 8, 9, 10, 11, 12};
    rt_fwd(q2);
    CHECK(q2[1] == 5 && q2[2] == 5, "rt_fwd forwarding");

    long xs[4] = {100, 200, 300, 400};
    rt_xseed(xs, xs);   // aliasing: r and q are the same memory
    // r[0]=101, r[1]=201 first; then c was 100 (pre-store), d = q[1] = 201
    // (post-store); q[0]=101, q[1]=202.
    CHECK(xs[0] == 101 && xs[1] == 202, "rt_xseed aliasing");

    long pr[8] = {1, 2, 3, 4, 5, 6, 7, 8};
    rt_precise(pr);
    CHECK(pr[0] == 2 && pr[1] == 3 && pr[2] == 4 && pr[3] == 5, "rt_precise values");

    long qp[2] = {7, 9};
    CHECK(rt_phi(qp, 1, 1, 1) == 8 + 10, "rt_phi taken");
    CHECK(rt_phi(qp, 0, 1, 1) == 0, "rt_phi fallthrough");

    long xq[4] = {1000000000L, -2000000000L, 0, 0};
    long xr[4] = {123456789L, 987654321L, 0, 0};
    long xa = 1000000000L + 123456789L, xbb = -2000000000L + 987654321L;
    CHECK(rt_xblock(xq, xr, 1) == xa * xbb, "rt_xblock taken");
    CHECK(rt_xblock(xq, xr, 0) == xa - xbb, "rt_xblock fallthrough");
    CHECK(xq[2] == xa && xq[3] == xbb, "rt_xblock stores");

    long qa[2] = {4, 6};
    CHECK(rt_asm(qa, 10, 20) == 14 + 26, "rt_asm value");

    long qat[2] = {3, 5};
    atomic_llong slot = 0;
    rt_atomic(qat, 100, 200, &slot);
    CHECK(slot == 205 && qat[0] == 103 && qat[1] == 205, "rt_atomic");

    H20 h_in, h_out;
    for (int k = 0; k < 20; k++) h_in.h[k] = (uint16_t)(k * 10);
    rt_w16(&h_out, &h_in);
    CHECK(h_out.h[0] == 1 && h_out.h[19] == 191, "rt_w16 values");

    H8nr hn_in, hn_out;
    for (int k = 0; k < 8; k++) hn_in.h[k] = (uint16_t)(k * 3);
    rt_w16_norestr(&hn_out, &hn_in);
    CHECK(hn_out.h[0] == 1 && hn_out.h[7] == 22, "rt_w16_norestr values");

    B34 b_in, b_out;
    for (int k = 0; k < 34; k++) b_in.b[k] = (uint8_t)(k * 7);
    rt_w8(&b_out, &b_in);
    CHECK(b_out.b[0] == 1 && b_out.b[33] == (uint8_t)(33 * 7 + 1), "rt_w8 values");

    printf(fails ? "bb_slp_redteam FAILURES: %d\n" : "bb_slp_redteam: all pass (%d fails)\n", fails);
    return fails != 0;
}
