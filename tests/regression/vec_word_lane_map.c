/*
 * 16-bit lane map vectorization (OP-05g) — exhaustive gate.
 *
 * The demotion machinery of OP-05d is width-parametric: truncation to 16
 * bits is a ring homomorphism for exactly the same reason as truncation to
 * 8, and the compare side conditions are the same two domains one width up
 * ([0, 65535] and [-32768, 32767], from `narrow_domains`).  This file pins
 * the WIDTH-SPECIFIC facts that the byte tests cannot cover:
 *
 *  * `Mul` IS admitted at 16-bit lanes (`pmullw` keeps the low half, exact
 *    for both signednesses) while it is refused at 8 (no packed byte
 *    multiply exists) — `w_scale`/`w_mulwrap`;
 *  * signed word min/max are SSE2 BASELINE (`pminsw`/`pmaxsw`), unlike both
 *    the byte and the dword signed forms, so `w_clamp` must fold at BOTH
 *    vector widths;
 *  * unsigned word min/max are SSE4.1, so `w_uclamp` folds only under AVX2
 *    and must stay exact via compare+blend at the baseline;
 *  * a word compare sets all 16 bits of its lane, which is what makes the
 *    per-BYTE `vpblendvb` exact at word granularity — `w_sel`;
 *  * the unsigned range fusion and the single-bit window union apply at 16
 *    bits unchanged — `w_class`, `w_union`.
 *
 * Deliberate wraparound (`w_mulwrap`, `w_addwrap`) proves the homomorphism
 * rather than just the in-range case.  Every trip count from 0 up is run so
 * the packed body, the scalar remainder and the empty loop are covered, and
 * every reference is computed through `volatile` scalars.
 */
#include <stdio.h>
#include <string.h>
#include <stdlib.h>

#define N 1031

static unsigned short usrc[N], udst[N], uref[N];
static short          ssrc[N], sdst[N], sref[N];
static int fails;

void w_clamp(short *restrict d, const short *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) { short v = s[i];
        if (v < -4000) v = -4000; if (v > 4000) v = 4000; d[i] = v; }
}
void w_uclamp(unsigned short *restrict d, const unsigned short *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) { unsigned short v = s[i];
        if (v < 1000u) v = 1000u; if (v > 60000u) v = 60000u; d[i] = v; }
}
void w_scale(short *restrict d, const short *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) d[i] = (short)(s[i] * 3 + 7);
}
void w_mulwrap(unsigned short *restrict d, const unsigned short *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) d[i] = (unsigned short)(s[i] * 40503u);
}
void w_addwrap(unsigned short *restrict d, const unsigned short *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) d[i] = (unsigned short)(s[i] + 60000u);
}
void w_class(unsigned short *restrict d, const unsigned short *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) { unsigned short c = s[i];
        d[i] = (unsigned short)((c >= 1000u && c <= 2000u) ? 1 : 0); }
}
void w_union(unsigned short *restrict d, const unsigned short *restrict s, unsigned long n) {
    /* two windows one bit apart -> single-bit window union at 16 bits */
    for (unsigned long i = 0; i < n; ++i) { unsigned short c = s[i];
        d[i] = (unsigned short)(((c >= 256u && c <= 300u) || (c >= 512u && c <= 556u)) ? 1 : 0); }
}
void w_sel(short *restrict d, const short *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) { short v = s[i]; d[i] = v < 0 ? (short)-v : (short)(v + 1); }
}
void w_rt(unsigned short *restrict d, const unsigned short *restrict s, unsigned long n, unsigned short k) {
    for (unsigned long i = 0; i < n; ++i) { unsigned short c = s[i];
        d[i] = (unsigned short)(c < k ? c + k : c - k); }
}
void w_shift_refuse(unsigned short *restrict d, const unsigned short *restrict s, unsigned long n) {
    /* no shift rule in the demotion yet: must refuse and stay exact */
    for (unsigned long i = 0; i < n; ++i) d[i] = (unsigned short)((s[i] >> 3) | (s[i] << 13));
}

#define CHK(NAME, DST, REF, M) \
    do { for (unsigned long q = 0; q < (M); ++q) if ((DST)[q] != (REF)[q]) { \
        printf("FAIL %s n=%lu i=%lu got=%ld want=%ld\n", NAME, (unsigned long)(M), q, \
               (long)(DST)[q], (long)(REF)[q]); if (++fails > 8) exit(1); break; } } while (0)

int main(void) {
    for (int i = 0; i < N; ++i) {
        usrc[i] = (unsigned short)(i * 2654435761u + (unsigned)i);
        ssrc[i] = (short)usrc[i];
    }
    /* domain edges at low indices so short trip counts still see them */
    unsigned short edges[] = {0, 1, 999, 1000, 1001, 2000, 2001, 255, 256, 300, 301,
                              511, 512, 556, 557, 32767, 32768, 32769, 59999, 60000,
                              60001, 65534, 65535};
    for (unsigned i = 0; i < sizeof edges / sizeof edges[0]; ++i) {
        usrc[i] = edges[i];
        ssrc[i] = (short)edges[i];
    }
    ssrc[0] = -32768; ssrc[1] = 32767; ssrc[2] = -4001; ssrc[3] = -4000;
    ssrc[4] = 4000; ssrc[5] = 4001; ssrc[6] = 0; ssrc[7] = -1;

    static const unsigned short ks[] = {0, 1, 1000, 32767, 32768, 65535};
    for (unsigned long n = 0; n <= N; n = (n < 70 ? n + 1 : n * 3 + 1)) {
        unsigned long M = n > N ? N : n;
#define RUNU(FN, EXPR) do { memset(udst, 0xAB, sizeof udst); \
        for (unsigned long i = 0; i < M; ++i) { volatile unsigned short vv = usrc[i]; \
            unsigned short c = vv; uref[i] = (unsigned short)(EXPR); } \
        FN(udst, usrc, M); CHK(#FN, udst, uref, M); } while (0)
#define RUNS(FN, EXPR) do { memset(sdst, 0xAB, sizeof sdst); \
        for (unsigned long i = 0; i < M; ++i) { volatile short vv = ssrc[i]; \
            short v = vv; sref[i] = (short)(EXPR); } \
        FN(sdst, ssrc, M); CHK(#FN, sdst, sref, M); } while (0)

        RUNS(w_clamp,  v < -4000 ? -4000 : (v > 4000 ? 4000 : v));
        RUNU(w_uclamp, c < 1000u ? 1000u : (c > 60000u ? 60000u : c));
        RUNS(w_scale,  v * 3 + 7);
        RUNU(w_mulwrap, c * 40503u);
        RUNU(w_addwrap, c + 60000u);
        RUNU(w_class,  (c >= 1000u && c <= 2000u) ? 1 : 0);
        RUNU(w_union,  ((c >= 256u && c <= 300u) || (c >= 512u && c <= 556u)) ? 1 : 0);
        RUNS(w_sel,    v < 0 ? -v : v + 1);
        RUNU(w_shift_refuse, (c >> 3) | (c << 13));

        for (unsigned ki = 0; ki < sizeof ks / sizeof ks[0]; ++ki) {
            unsigned short k = ks[ki];
            memset(udst, 0xAB, sizeof udst);
            for (unsigned long i = 0; i < M; ++i) { volatile unsigned short vv = usrc[i];
                unsigned short c = vv; uref[i] = (unsigned short)(c < k ? c + k : c - k); }
            w_rt(udst, usrc, M, k); CHK("w_rt", udst, uref, M);
        }
    }
    if (fails) { puts("VALIDATION FAILED"); return 1; }
    puts("VALIDATION OK");
    return 0;
}
