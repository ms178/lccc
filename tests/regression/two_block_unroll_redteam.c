/*
 * Two-block partial unroller + half-wide SLP pairs red-team battery:
 * the session-53 feature set's adversarial edges.
 *
 * Every kernel is verified against an independently computed reference
 * (scalar accumulators kept in locals the optimizer cannot rewrite into
 * the vector form, or closed-form arithmetic). The whole battery is run
 * tri-config by the gate script and must be byte-identical under
 * (a) everything on, (b) CCC_NO_TWO_BLOCK_UNROLL=1 CCC_NO_BB_SLP=1 (the
 * rolled scalar reference of the same compiler), and (c) gcc -O2
 * -march=x86-64-v3 (the independent oracle).
 *
 * Coverage map (each class documented at its kernel):
 *   A. CARRIED-PHI/IV INTERACTION — the "x = i" miscompile class (a
 *      carried phi whose back edge references the IV phi itself must
 *      thread through the per-clone IVs, not the group-start IV), the
 *      GVN-shaped "x = i + 1" (back edge = a latch Add), cross-phi
 *      threading pairs, const-backed carried phis, live-out exactness
 *      of IV and carried values, negative-step countdown IVs, u8/u32/
 *      u64 IV widths, !=/<=/>= exit spellings, odd trips (divisibility
 *      decline), trip == 2k (minimum two groups), nested two-block
 *      loops, volatile body traffic.
 *      PROFITABILITY CONTRACT: every A-kernel loop carries non-volatile
 *      load+store feedstock (an `o[i] = o[i] + <carried>` RMW) — the
 *      two-block unroller exists to manufacture BB-SLP feedstock, so a
 *      latch without both declines (measured: gzip_crc32 +30% insns,
 *      glibc_memcmp +20.7%, crc32 fill_data +21 lines, all at neutral
 *      runtime). The RMW's store depends on the carried phi, so the
 *      loop stays non-distributable and the vectorizer keeps declining
 *      it — the partial unroller is the only transform here.
 *   B. BARE-VALUE POSITIONS — Store.ptr / GEP.base threading (the
 *      memset-escape class: a phi pointer stored through, plus the
 *      fill-loop shape with the IV live-out past the loop).
 *   C. MEMORY-ORDER EXACTNESS — store-to-load forwarding shapes the
 *      SLP must reject per-seed but the unroller must still order
 *      correctly (m[i] = f(m[i-1])), cross-iteration dependences at
 *      distance 2 and 3, mixed load/store windows.
 *   D. HALF-WIDE PAIRS + STREAM CSE — the message-schedule shape
 *      (pair loads re-loaded across a disjoint store window must CSE;
 *      across an OVERLAPPING store must NOT), pair stores of computed
 *      dword pairs, SIB index-var gathers.
 *   E. LOOP-VEC GATES — max reduction (find_max), conditional-sum
 *      (masked accumulate), max of negatives/duplicates/first-wins.
 *   F. PEEPHOLES — the hash-chain walk (cmp with the loaded scratch in
 *      the destination slot), the induction copy-back (list-walk
 *      pointer chase with the pointer re-used after the copy), the
 *      read-write window hazard (the copied register read AFTER a
 *      read-write line must see the new value), dead-leaf frames
 *      (correctness of the frame elision under calls), BMI andn
 *      correctness.
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
        }                                                                    \
    } while (0)

/* ═════════════════════════════════════════════════════════════════════════
 * A. Carried-phi / IV interaction
 * ═════════════════════════════════════════════════════════════════════════ */

/* A1: THE "x = i" MISCOMPILE CLASS. A carried phi X whose back edge is
 * the IV phi itself (the C `x = i;`). Under a xk unroll, clone j >= 2
 * must read the IV of clone j-1 (not the group-start IV), and the
 * loop's live-out X must be the LAST iteration's i. This exact shape
 * measured x = 60 (the last group's start IV, k = 4) instead of 63
 * before the threading fix. The multiply by X declines the loop
 * vectorizer, so the partial unroller is the only transform here; the
 * o-RMW is the load+store feedstock the profitability gate requires
 * (its store depends on X, keeping the loop non-distributable). */
static int o1[64];
static void a1_carried_phi_is_iv(void) {
    int a[64], s = 0, x = 5, ref = 0, refx = 5;
    for (int i = 0; i < 64; i++) a[i] = i * 3 + 1;
    for (int i = 0; i < 64; i++) {
        o1[i] = o1[i] + x;
        s += a[i] * x;
        x = i;
    }
    /* independent reference: replay the same recurrence */
    {
        int xx = 5;
        for (int i = 0; i < 64; i++) {
            ref += a[i] * xx;
            xx = i;
        }
        refx = xx;
    }
    CHECK(s == ref && x == refx && x == 63, "A1 carried-phi-is-IV");
    /* the RMW mirror: o1[i] = X at iteration i (5 initially, then i-1) */
    {
        int ok = (o1[0] == 5);
        for (int i = 1; i < 64; i++) ok = ok && (o1[i] == i - 1);
        CHECK(ok, "A1b carried-phi-is-IV RMW feedstock");
    }
}

/* A2: the GVN-shaped "x = i + 1": the back edge is a latch Add (either
 * the user's own i+1 or one CSE'd with the canonical increment). Live
 * out must be the post-increment value of the LAST iteration. */
static int o2[64];
static void a2_carried_phi_iv_plus_one(void) {
    int a[64], s = 0, x = 5, ref = 0, refx = 5;
    for (int i = 0; i < 64; i++) a[i] = i * 5 - 2;
    for (int i = 0; i < 64; i++) {
        o2[i] = o2[i] + x;
        s += a[i] ^ x;
        x = i + 1;
    }
    {
        int xx = 5;
        for (int i = 0; i < 64; i++) {
            ref += a[i] ^ xx;
            xx = i + 1;
        }
        refx = xx;
    }
    CHECK(s == ref && x == refx && x == 64, "A2 carried-phi-iv+1");
    /* o2[i] = X at iteration i (5 initially, then i) */
    {
        int ok = (o2[0] == 5);
        for (int i = 1; i < 64; i++) ok = ok && (o2[i] == i);
        CHECK(ok, "A2b carried-phi-iv+1 RMW feedstock");
    }
}

/* A3: cross-phi threading (two rotation edges — under the >= 3 gate):
 * g trails f by one iteration. Both live out. */
static int o3[48];
static void a3_cross_phi_pair(void) {
    int f = 2, g = 1, sf = 0, sg = 0, reff = 2, refg = 1, sreff = 0, srefg = 0;
    for (int i = 0; i < 48; i++) {
        int nf = f + g + i;
        o3[i] = o3[i] + g;
        g = f;
        f = nf;
        sf += f;
        sg += g;
    }
    {
        int ff = 2, gg = 1, exp3[48];
        for (int i = 0; i < 48; i++) {
            int nf = ff + gg + i;
            exp3[i] = gg;
            gg = ff;
            ff = nf;
            sreff += ff;
            srefg += gg;
        }
        reff = ff;
        refg = gg;
        int ok3 = 1;
        for (int i = 0; i < 48; i++) ok3 = ok3 && (o3[i] == exp3[i]);
        CHECK(ok3, "A3b cross-phi RMW feedstock");
    }
    CHECK(sf == sreff && sg == srefg && f == reff && g == refg,
          "A3 cross-phi pair (2 rotation edges)");
}

/* A4: const-backed carried phi: `c = 0;` reset at the end of every
 * iteration (back edge = Const). Clone j >= 1 reads the constant.
 * (The const-backed phi minted before the IV is also the
 * IV-finder-order shape — a const back edge must not hide the IV.) */
static int o4[40];
static void a4_const_backed_phi(void) {
    int c = 7, s = 0, refc = 7, refs = 0;
    for (int i = 0; i < 40; i++) {
        o4[i] = o4[i] + c;
        s += c * (i + 1);
        c = 0;
    }
    {
        int cc = 7;
        for (int i = 0; i < 40; i++) {
            refs += cc * (i + 1);
            cc = 0;
        }
        refc = cc;
    }
    CHECK(s == refs && c == refc && c == 0, "A4 const-backed carried phi");
    /* o4[i] = C at iteration i (7 initially, then 0) */
    {
        int ok = (o4[0] == 7);
        for (int i = 1; i < 40; i++) ok = ok && (o4[i] == 0);
        CHECK(ok, "A4b const-backed-phi RMW feedstock");
    }
}

/* A5: mixed carried set: accumulator + iv-valued phi + const phi, all
 * live out, trip 96 (divisible by 2, 4 and 8 — exercises the k walk). */
static int o5[96];
static void a5_mixed_carried_set(void) {
    int a[96], s = 0, last = -1, seen = 1, refs = 0, reflast = -1, refseen = 1;
    for (int i = 0; i < 96; i++) a[i] = (i * 37) % 101;
    for (int i = 0; i < 96; i++) {
        o5[i] = o5[i] + last;
        s += a[i];
        last = i;
        seen = 0;
    }
    for (int i = 0; i < 96; i++) {
        refs += a[i];
        reflast = i;
        refseen = 0;
    }
    CHECK(s == refs && last == reflast && last == 95 && seen == refseen,
          "A5 mixed carried set live-out");
    /* o5[i] = last at iteration i (-1 initially, then i-1) */
    {
        int ok = (o5[0] == -1);
        for (int i = 1; i < 96; i++) ok = ok && (o5[i] == i - 1);
        CHECK(ok, "A5b mixed-carried RMW feedstock");
    }
}

/* A6: negative-step countdown IV (`i -= 4`), >= exit spelling. Trip 24
 * (92, 88, ..., 0) — divisible by 4, so the unroll fires on the
 * negative step. */
static int o6[96];
static void a6_countdown(void) {
    int s = 0, x = 0, ref = 0, refx = 0;
    for (int i = 92; i >= 0; i -= 4) {
        o6[i] = o6[i] + x;
        s += i;
        x = i;
    }
    for (int i = 92; i >= 0; i -= 4) {
        ref += i;
        refx = i;
    }
    CHECK(s == ref && x == refx && x == 0, "A6 countdown IV, >= exit");
    /* o6[i] = X at iteration i (0 initially, then i+4) */
    {
        int ok = (o6[92] == 0);
        for (int i = 88; i >= 0; i -= 4) ok = ok && (o6[i] == i + 4);
        CHECK(ok, "A6b countdown RMW feedstock");
    }
}

/* A7: u8 and u64 IV widths. */
static unsigned char o7c[200];
static uint64_t o7l[100];
static void a7_iv_widths(void) {
    unsigned char i8v;
    uint64_t s64 = 0, ref64 = 0;
    int s8 = 0, ref8 = 0;
    unsigned char last8 = 0, reflt8 = 0;
    for (i8v = 0; i8v < 200; i8v++) { /* trip 200: > complete-unroll cap */
        o7c[i8v] = (unsigned char)(o7c[i8v] + last8);
        s8 += i8v;
        last8 = i8v;
    }
    for (unsigned char k = 0; k < 200; k++) {
        ref8 += k;
        reflt8 = k;
    }
    CHECK(s8 == ref8 && last8 == reflt8, "A7a u8 IV");
    /* o7c[i] = last8 at iteration i (0 initially, then i-1) */
    {
        int ok = (o7c[0] == 0);
        for (int i = 1; i < 200; i++)
            ok = ok && (o7c[i] == (unsigned char)(i - 1));
        CHECK(ok, "A7a2 u8 RMW feedstock");
    }
    for (uint64_t i = 0; i < 100; i++) {
        s64 += i * 0x9E3779B97F4A7C15ULL;
        o7l[i] = o7l[i] + s64;
    }
    for (uint64_t i = 0; i < 100; i++) {
        ref64 += i * 0x9E3779B97F4A7C15ULL;
    }
    CHECK(s64 == ref64, "A7b u64 IV");
    {
        uint64_t r = 0;
        int ok = 1;
        for (uint64_t i = 0; i < 100; i++) {
            r += i * 0x9E3779B97F4A7C15ULL;
            ok = ok && (o7l[i] == r);
        }
        CHECK(ok, "A7b2 u64 RMW feedstock");
    }
}

/* A8: != and <= exit spellings. */
static int o8a[70], o8b[70];
static void a8_exit_spellings(void) {
    int s = 0, ref = 0, x = 0, refx = 0;
    for (int i = 3; i != 67; i++) { /* != exit, nonzero init */
        o8a[i] = o8a[i] + x;
        s += i;
        x = i;
    }
    for (int k = 3; k != 67; k++) {
        ref += k;
        refx = k;
    }
    CHECK(s == ref && x == refx && x == 66, "A8a != exit");
    /* o8a[i] = X at iteration i (0 initially, then i-1) */
    {
        int ok = (o8a[3] == 0);
        for (int i = 4; i < 67; i++) ok = ok && (o8a[i] == i - 1);
        CHECK(ok, "A8a2 != exit RMW feedstock");
    }
    s = ref = 0;
    for (int i = 0; i <= 65; i++) { /* <= exit: trip 66 */
        s += i;
        o8b[i] = o8b[i] + s;
    }
    for (int k = 0; k <= 65; k++) {
        ref += k;
    }
    CHECK(s == ref, "A8b <= exit");
    {
        int r = 0, ok = 1;
        for (int i = 0; i <= 65; i++) {
            r += i;
            ok = ok && (o8b[i] == r);
        }
        CHECK(ok, "A8b2 <= exit RMW feedstock");
    }
}

/* A9: odd trip (33) — no power-of-two k divides it, the unroller must
 * decline and the loop must still be exact. */
static void a9_odd_trip(void) {
    int s = 0, ref = 0, x = 0, refx = 0;
    for (int i = 0; i < 33; i++) {
        s += i * i;
        x = i;
    }
    for (int k = 0; k < 33; k++) {
        ref += k * k;
        refx = k;
    }
    CHECK(s == ref && x == refx && x == 32, "A9 odd trip declines");
}

/* A10: SMALL-TRIP ROUTING. The old comment's "trip == 2k minimum,
 * exactly two groups" described a Pass-B edge that is complete-unroll
 * territory by construction: k is capped at 4, so trip == 2k <= 8 <=
 * the complete unroller's trip cap (16), and Pass A's budget accepts
 * any body Pass B would look at. Verified: this kernel is flattened
 * and constant-folded away entirely (the CHECK folds to true; the
 * function vanishes from the -S output). It now pins the
 * complete-unroll + constant-folding exactness of the carried phi. */
static void a10_min_groups(void) {
    int a[8], s = 0, x = 9, ref = 0, refx = 9;
    for (int i = 0; i < 8; i++) a[i] = i * 11 + 3;
    for (int i = 0; i < 8; i++) {
        s += a[i] * x;
        x = i;
    }
    {
        int xx = 9;
        for (int i = 0; i < 8; i++) {
            ref += a[i] * xx;
            xx = i;
        }
        refx = xx;
    }
    CHECK(s == ref && x == refx && x == 7, "A10 small-trip complete-unroll");
}

/* A11: nested two-block loops (inner unrolled, outer must stay intact
 * — and the IV threading must not confuse the two). */
static int o11[40];
static void a11_nested(void) {
    int s = 0, ref = 0;
    for (int j = 0; j < 12; j++) {
        int inner = 0, ix = -1;
        for (int i = 0; i < 40; i++) {
            inner += (i + j) * (ix < 0 ? 1 : ix);
            o11[i] = o11[i] + ix;
            ix = i;
        }
        s += inner;
    }
    for (int j = 0; j < 12; j++) {
        int inner = 0, ix = -1;
        for (int i = 0; i < 40; i++) {
            inner += (i + j) * (ix < 0 ? 1 : ix);
            ix = i;
        }
        ref += inner;
    }
    CHECK(s == ref, "A11 nested two-block loops");
    /* o11[i] accumulates ix over 12 outer rounds: -12, then 12*(i-1) */
    {
        int ok = (o11[0] == -12);
        for (int i = 1; i < 40; i++) ok = ok && (o11[i] == 12 * (i - 1));
        CHECK(ok, "A11b nested inner RMW feedstock");
    }
}

/* A12: volatile body traffic riding packable work: the plain RMW is
 * the feedstock that keeps the unroll firing; the volatile load and
 * store must still be cloned per iteration and read/written exactly
 * trip times, in order. */
static int vol_sink;
static volatile int vol_src = 3;
static int o12[36];
static void a12_volatile(void) {
    int s = 0, x = 0, ref = 0, refx = 0;
    for (int i = 0; i < 36; i++) {
        s += vol_src;
        vol_sink = i;
        o12[i] = o12[i] + x;
        x = i;
    }
    for (int k = 0; k < 36; k++) {
        ref += 3;
        refx = k;
    }
    CHECK(s == ref && x == refx && vol_sink == 35, "A12 volatile body");
    {
        int ok = (o12[0] == 0);
        for (int i = 1; i < 36; i++) ok = ok && (o12[i] == i - 1);
        CHECK(ok, "A12b mixed volatile/plain body");
    }
}

/* ═════════════════════════════════════════════════════════════════════════
 * B. Bare-value positions (Store.ptr / GEP.base threading)
 * ═════════════════════════════════════════════════════════════════════════ */

/* B1: the fill-loop + IV escape: the store's address is a latch-defined
 * GEP dest (Store.ptr is a bare-Value position for_each_operand_mut
 * skips — the per-clone rename must cover it), and the IV is live-out
 * past the loop (materialised, not assumed). The fill is an RMW
 * (load+store feedstock); trip 76 = 4*19 so the unroll fires (77 was
 * prime — the old shape silently declined). */
static void b1_fill_iv_escape(void) {
    static int buf[76];
    int n = 0;
    for (int i = 0; i < 76; i++) {
        buf[i] += i * 2 + 1;
        n = i;
    }
    int ok = (n == 75);
    long sum = 0;
    for (int i = 0; i < 76; i++) sum += buf[i];
    /* sum of (2i+1), i<76 = 76^2 (the odd-number identity) */
    CHECK(ok && sum == 5776, "B1 fill loop, IV escape");
}

/* B2: byte-fill through a moving pointer (char stores, pointer phi in
 * Store.ptr AND Load.ptr — the bare-Value positions). The INTEGER IV
 * drives the counted trip (a pointer phi is not an IV for either
 * unroller); the pointer advances in parallel and every access goes
 * through it. Trip 128 = 4*32 so the unroll fires (131 was prime — the
 * old shape silently declined). */
static void b2_byte_fill(void) {
    static unsigned char buf[128];
    unsigned char *p = buf;
    for (int i = 0; i < 128; i++) {
        *p = (unsigned char)(*p + i * 7);
        p++;
    }
    unsigned acc = 0;
    for (int i = 0; i < 128; i++) acc = acc * 31 + buf[i];
    unsigned ref = 0;
    for (int i = 0; i < 128; i++) ref = ref * 31 + (unsigned char)(i * 7);
    CHECK(acc == ref, "B2 byte fill via pointer phi");
}

/* ═════════════════════════════════════════════════════════════════════════
 * C. Memory-order exactness under store-to-load forwarding shapes
 * ═════════════════════════════════════════════════════════════════════════ */

/* C1: m[i] = f(m[i-1]) — the store→load-forwarding shape: SLP rule (e)
 * must reject the seed (the load window overlaps the prior store), so
 * the pair pack declines, but the UNROLLED clones must still order the
 * dependence exactly (this is the class where a wrong unroll would read
 * the new value one iteration early). Trip 70 = 2*35 (the old 69 was
 * odd and silently declined — the recurrence never unrolled). */
static void c1_forwarding_distance1(void) {
    int m[72];
    for (int i = 0; i < 72; i++) m[i] = i;
    for (int i = 1; i < 71; i++) {
        m[i] = m[i - 1] + 3;
    }
    int ref = 0;
    {
        int mm[72];
        for (int i = 0; i < 72; i++) mm[i] = i;
        for (int i = 1; i < 71; i++) mm[i] = mm[i - 1] + 3;
        for (int i = 0; i < 72; i++) ref += mm[i] * (i + 1);
    }
    int acc = 0;
    for (int i = 0; i < 72; i++) acc += m[i] * (i + 1);
    CHECK(acc == ref, "C1 forwarding distance 1");
}

/* C2: distance-2 recurrence (the schedule shape: m[i] = m[i-2] op …)
 * with a disjoint seed store in the window (the stream-CSE allowed
 * case) — values exact. */
static void c2_schedule_shape(void) {
    uint32_t m[72];
    for (int i = 0; i < 8; i++) m[i] = (uint32_t)(i * 2654435761u);
    for (int i = 8; i < 72; i++) {
        uint32_t x = m[i - 2];
        uint32_t y = m[i - 5];
        m[i] = (x ^ (x >> 3)) + (y << 5) + (uint32_t)i;
    }
    uint32_t ref = 0;
    {
        uint32_t mm[72];
        for (int i = 0; i < 8; i++) mm[i] = (uint32_t)(i * 2654435761u);
        for (int i = 8; i < 72; i++) {
            uint32_t x = mm[i - 2];
            uint32_t y = mm[i - 5];
            mm[i] = (x ^ (x >> 3)) + (y << 5) + (uint32_t)i;
        }
        for (int i = 0; i < 72; i++) ref = ref * 131 + mm[i];
    }
    uint32_t acc = 0;
    for (int i = 0; i < 72; i++) acc = acc * 131 + m[i];
    CHECK(acc == ref, "C2 schedule shape distance 2/5");
}

/* C3: the OVERLAPPING re-load: the same stream re-read AFTER a store
 * that covers the re-read window must see the NEW bytes (the CSE must
 * be blocked; forwarding would silently pick the old value). */
static void c3_overlapping_reload(void) {
    uint32_t m[24];
    for (int i = 0; i < 24; i++) m[i] = (uint32_t)i * 100;
    uint32_t first, second;
    /* one iteration shape: read the pair, overwrite it, read it again */
    for (int i = 8; i < 16; i += 2) {
        uint32_t a = m[i] ^ m[i + 1];
        m[i] = a;
        m[i + 1] = a + 1;
        /* re-read the just-written window: must see the new values */
        uint32_t b = m[i] + m[i + 1];
        first = a;
        second = b;
    }
    CHECK(second == 2 * first + 1, "C3 overlapping re-load sees new bytes");
}

/* ═════════════════════════════════════════════════════════════════════════
 * D. Half-wide pairs + stream CSE + SIB
 * ═════════════════════════════════════════════════════════════════════════ */

/* D1: the pair-store shape: two consecutive dword stores of computed
 * values. The pack2 family proper is the pair-LOAD form (see d4's
 * contract); this kernel pins the exactness of adjacent computed
 * dword stores — which stay as separate 32-bit stores by design (the
 * stored lanes are not load-derived, so no seed forms). */
static void d1_pair_stores(void) {
    static uint32_t dst[64];
    for (int i = 0; i < 64; i += 2) {
        uint32_t x = (uint32_t)i * 3 + 1;
        uint32_t y = (uint32_t)i * 5 + 2;
        dst[i] = x ^ 0xAAAA;
        dst[i + 1] = y ^ 0x5555;
    }
    uint32_t acc = 0;
    for (int i = 0; i < 64; i++) acc = acc * 17 + dst[i];
    uint32_t ref = 0;
    for (int i = 0; i < 64; i += 2) {
        uint32_t x = (uint32_t)i * 3 + 1;
        uint32_t y = (uint32_t)i * 5 + 2;
        ref = ref * 17 + (x ^ 0xAAAA);
        ref = ref * 17 + (y ^ 0x5555);
    }
    CHECK(acc == ref, "D1 dword-pair stores");
}

/* D2: pair loads feeding pair ops (rotate idiom at pair width): the
 * five re-loads of one window across a disjoint store must CSE — but
 * exactness is the contract here. */
static uint32_t d2_buf[96];
static void d2_pair_ops(void) {
    for (int i = 0; i < 96; i++) d2_buf[i] = (uint32_t)(i * 7 + 3);
    for (int i = 16; i < 96; i++) {
        uint32_t x = d2_buf[i - 2];
        uint32_t y = d2_buf[i - 7];
        uint32_t r = (x << 4) | (x >> 28);
        d2_buf[i] = r ^ y ^ (uint32_t)i;
    }
    uint32_t ref = 0;
    {
        uint32_t mm[96];
        for (int i = 0; i < 96; i++) mm[i] = (uint32_t)(i * 7 + 3);
        for (int i = 16; i < 96; i++) {
            uint32_t x = mm[i - 2];
            uint32_t y = mm[i - 7];
            uint32_t r = (x << 4) | (x >> 28);
            mm[i] = r ^ y ^ (uint32_t)i;
        }
        for (int i = 0; i < 96; i++) ref = ref * 33 + mm[i];
    }
    uint32_t acc = 0;
    for (int i = 0; i < 96; i++) acc = acc * 33 + d2_buf[i];
    CHECK(acc == ref, "D2 pair ops with rotate idiom");
}

/* D3: SIB gather: a[idx[i]] reads with idx a loop-carried array (the
 * index-variable stream decomposition). The od-RMW is the store
 * feedstock that keeps the unroll firing (the old load-only latch
 * declined under the profitability gate and the SIB pairs never
 * formed). */
static int od3[64];
static void d3_sib_gather(void) {
    static const int tab[256] = {0};
    int idx[64], s = 0, ref = 0;
    for (int i = 0; i < 64; i++) idx[i] = (i * 37) % 256;
    /* tab is const-zero; fold in i to make the sum observable */
    for (int i = 0; i < 64; i++) {
        s += tab[idx[i]] + i;
        od3[i] = od3[i] + s;
    }
    for (int i = 0; i < 64; i++) ref += i;
    CHECK(s == ref, "D3 SIB gather");
    {
        int r = 0, ok = 1;
        for (int i = 0; i < 64; i++) {
            r += i;
            ok = ok && (od3[i] == r);
        }
        CHECK(ok, "D3b SIB gather RMW feedstock");
    }
}

/* D4: THE MESSAGE-SCHEDULE PAIR SHAPE (the sha256 2-wide form the
 * half-wide family exists for): adjacent pair loads, rotate/xor/add
 * lane ops, ONE pair store per iteration. Values exact in all three
 * configs; the gate script additionally asserts the vmovq pair loads
 * and VEX dword ops (no legacy SSE mixing) on this kernel. */
static uint32_t d4_w[80];
static void d4_schedule_pairs(void) {
    static const uint32_t init[16] = {
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f,
        0x9b05688c, 0x1f83d9ab, 0x5be0cd19, 0x428a2f98, 0x71374491,
        0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5,
    };
    for (int i = 0; i < 16; i++) d4_w[i] = init[i];
    for (int i = 16; i < 64; i++) {
        uint32_t x = d4_w[i - 15];
        uint32_t y = d4_w[i - 2];
        uint32_t s0 = (x >> 7) | (x << 25);
        s0 ^= (x >> 18) | (x << 14);
        s0 ^= x >> 3;
        uint32_t s1 = (y >> 17) | (y << 15);
        s1 ^= (y >> 19) | (y << 13);
        s1 ^= y >> 10;
        d4_w[i] = d4_w[i - 16] + s0 + d4_w[i - 7] + s1;
    }
    uint32_t ref = 0;
    {
        uint32_t m[64];
        for (int i = 0; i < 16; i++) m[i] = init[i];
        for (int i = 16; i < 64; i++) {
            uint32_t x = m[i - 15];
            uint32_t y = m[i - 2];
            uint32_t s0 = (x >> 7) | (x << 25);
            s0 ^= (x >> 18) | (x << 14);
            s0 ^= x >> 3;
            uint32_t s1 = (y >> 17) | (y << 15);
            s1 ^= (y >> 19) | (y << 13);
            s1 ^= y >> 10;
            m[i] = m[i - 16] + s0 + m[i - 7] + s1;
        }
        for (int i = 0; i < 64; i++) ref = ref * 29 + m[i];
    }
    uint32_t acc = 0;
    for (int i = 0; i < 64; i++) acc = acc * 29 + d4_w[i];
    CHECK(acc == ref, "D4 message-schedule pair shape");
}

/* ═════════════════════════════════════════════════════════════════════════
 * E. Loop-vectorizer gates (Max reduction, conditional-sum)
 * ═════════════════════════════════════════════════════════════════════════ */

/* E1: find_max over mixed signs, duplicates, sentinel initial values. */
static void e1_find_max(void) {
    int a[197];
    for (int i = 0; i < 197; i++)
        a[i] = ((i * 73) % 251) - 125; /* mixed signs, duplicates */
    int mx = a[0];
    for (int i = 1; i < 197; i++)
        if (a[i] > mx) mx = a[i];
    int ref = a[0];
    for (int i = 1; i < 197; i++)
        if (a[i] > ref) ref = a[i];
    CHECK(mx == ref, "E1 find_max mixed");

    /* all-equal array: first-wins is fine for max (same value) */
    for (int i = 0; i < 197; i++) a[i] = -7;
    mx = a[0];
    for (int i = 1; i < 197; i++)
        if (a[i] > mx) mx = a[i];
    CHECK(mx == -7, "E1b find_max all equal");

    /* single element */
    a[0] = 42;
    mx = a[0];
    for (int i = 1; i < 1; i++)
        if (a[i] > mx) mx = a[i];
    CHECK(mx == 42, "E1c find_max single");
}

/* E2: conditional-sum (masked accumulate). */
static void e2_conditional_sum(void) {
    int a[163], s = 0, ref = 0;
    for (int i = 0; i < 163; i++) a[i] = ((i * 91) % 199) - 99;
    for (int i = 0; i < 163; i++)
        if (a[i] > 0) s += a[i] * 2;
    for (int i = 0; i < 163; i++)
        if (a[i] > 0) ref += a[i] * 2;
    CHECK(s == ref, "E2 conditional sum");

    /* none taken */
    for (int i = 0; i < 163; i++) a[i] = -i;
    s = 0;
    ref = 0;
    for (int i = 0; i < 163; i++)
        if (a[i] > 0) s += a[i];
    CHECK(s == ref && s == 0, "E2b conditional sum none");
}

/* ═════════════════════════════════════════════════════════════════════════
 * F. Backend peepholes (load_op_fuse, narrow_copy_fold, frame_compact)
 * ═════════════════════════════════════════════════════════════════════════ */

/* F1: hash-chain walk: the loaded scratch lands in the cmp DESTINATION
 * slot (the load_op_fuse mirror shape) — the fold must preserve the
 * pointer-vs-loaded comparison. */
static struct chain_node {
    struct chain_node *next;
    int key;
    int val;
} chain_nodes[48];
static void f1_hash_chain(void) {
    for (int i = 0; i < 48; i++) {
        chain_nodes[i].key = (i * 41) % 97;
        chain_nodes[i].val = i * 13;
        chain_nodes[i].next = (i + 1 < 48) ? &chain_nodes[i + 1] : 0;
    }
    int want = (29 * 41) % 97;
    int got = -1;
    for (struct chain_node *p = &chain_nodes[0]; p; p = p->next) {
        if (p->key == want) { /* cmp with loaded key in dst slot */
            got = p->val;
            break;
        }
    }
    CHECK(got == 29 * 13, "F1 hash-chain walk (cmp dst-slot fold)");
}

/* F2: the induction copy-back (narrow_copy_fold): pointer chase whose
 * staging register is re-used AFTER the copy (the read-write window
 * hazard: a later read of the copied register must see the post-copy
 * write, not the producer's value). */
static void f2_induction_copyback(void) {
    int cells[64], order[64];
    int head = 0;
    for (int i = 0; i < 63; i++) cells[i] = i + 1;
    cells[63] = -1;
    int n = 0;
    for (int p = head; p != -1; p = cells[p]) /* p's copy-back staging */
        order[n++] = p;
    int ok = (n == 64);
    for (int i = 0; i < 64; i++) ok = ok && (order[i] == i);
    CHECK(ok, "F2 induction copy-back list walk");
}

/* F3: the copy-back with the target re-used in address arithmetic
 * after a read-write (the cmp_replay_acc_nohome miscompile class). */
static void f3_copyback_readwrite(void) {
    static int tbl[128];
    for (int i = 0; i < 128; i++) tbl[i] = (i * 29) % 257;
    int idx = 3, s = 0, ref = 0, refidx = 3;
    for (int k = 0; k < 21; k++) {
        s += tbl[idx];
        idx = (idx + 17) & 127; /* read-modify-write of the index */
    }
    for (int k = 0; k < 21; k++) {
        ref += tbl[refidx];
        refidx = (refidx + 17) & 127;
    }
    CHECK(s == ref && idx == refidx, "F3 copy-back read-write window");
}

/* F4: dead-leaf frame elision under calls: a small helper called in a
 * loop from a function with a retired frame (the frame_compact shrink
 * path must keep the ABI alignment at every call site). */
static int f4_helper(int x) { return x * 3 + 1; }
static void f4_leaf_frames(void) {
    int s = 0, ref = 0;
    for (int i = 0; i < 53; i++) s += f4_helper(i);
    for (int i = 0; i < 53; i++) ref += i * 3 + 1;
    CHECK(s == ref, "F4 leaf frame elision with calls");
}

/* F5: BMI andn correctness (~x & y chains). */
static void f5_andn(void) {
    unsigned a = 0x0F0F0F0Fu, b = 0xFFFF00FFu, acc = 0;
    for (int i = 0; i < 31; i++) {
        acc = (~a) & b;
        a = (a << 1) | (a >> 31);
        b = ~(b << 3);
    }
    unsigned refa = 0x0F0F0F0Fu, refb = 0xFFFF00FFu, ref = 0;
    for (int i = 0; i < 31; i++) {
        ref = (~refa) & refb;
        refa = (refa << 1) | (refa >> 31);
        refb = ~(refb << 3);
    }
    CHECK(acc == ref, "F5 andn chain");
}

int main(void) {
    a1_carried_phi_is_iv();
    a2_carried_phi_iv_plus_one();
    a3_cross_phi_pair();
    a4_const_backed_phi();
    a5_mixed_carried_set();
    a6_countdown();
    a7_iv_widths();
    a8_exit_spellings();
    a9_odd_trip();
    a10_min_groups();
    a11_nested();
    a12_volatile();
    b1_fill_iv_escape();
    b2_byte_fill();
    c1_forwarding_distance1();
    c2_schedule_shape();
    c3_overlapping_reload();
    d1_pair_stores();
    d2_pair_ops();
    d3_sib_gather();
    d4_schedule_pairs();
    e1_find_max();
    e2_conditional_sum();
    f1_hash_chain();
    f2_induction_copyback();
    f3_copyback_readwrite();
    f4_leaf_frames();
    f5_andn();

    if (fails == 0) {
        printf("two_block_unroll_redteam: all pass (0 fails)\n");
        return 0;
    }
    printf("two_block_unroll_redteam: %d fails\n", fails);
    return 1;
}
