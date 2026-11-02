/*
 * Runtime byte invariants (`vpbroadcastb`) and signed byte min/max
 * (`vpminsb`/`vpmaxsb`) -- OP-05f -- plus the confluence fix in
 * `fold_int_minmax`.
 *
 * Three properties are pinned here:
 *
 *  1. A byte map whose invariant is a RUNTIME value (not a literal) is
 *     vectorized.  Constants splat for free through the dword broadcast of
 *     `b * 0x01010101`; a runtime byte needs a real `vpbroadcastb` (AVX2) or
 *     the SSE2 `movd`+`punpcklbw`+`punpcklwd`+`pshufd` chain.  Before this
 *     op existed the byte parser failed closed on every such loop.
 *
 *  2. The runtime invariant's RANGE is recovered through its integer
 *     promotion (`zext u8 -> i32`), which is what lets it appear as a
 *     compare operand.  Without that recovery `c < threshold` with a runtime
 *     `unsigned char` threshold has an unknown range and the compare side
 *     condition fails, keeping the loop scalar.
 *
 *  3. Nested clamps fold completely.  `fold_int_minmax` used to fold a
 *     select's ARMS but not its CONDITION's operands, so the inner clamp of
 *     `if (v<lo) v=lo; if (v>hi) v=hi;` reached the emitter in two different
 *     shapes: one folded to min/max, one still a compare+blend.  The emit
 *     cache keys on tree shape, so the body computed the inner clamp TWICE
 *     and spilled.  Both the byte and the dword clamp are checked, because
 *     the defect predated the byte path.
 *
 * Every kernel is run at every trip count from 0 up (packed body, scalar
 * remainder, empty loop) and against `volatile`-fed scalar references, with
 * runtime parameters chosen to include the domain edges (0, 1, 127, 128,
 * 254, 255 and the signed extremes).
 */
#include <stdio.h>
#include <string.h>
#include <stdlib.h>

#define N 1031

static unsigned char usrc[N], udst[N], uref[N];
static signed char   ssrc[N], sdst[N], sref[N];
static int           isrc[N], idst[N], iref[N];

/* ---- 1 & 2: runtime byte invariants --------------------------------- */
void rt_add(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n, unsigned char k) {
    for (unsigned long i = 0; i < n; ++i) d[i] = (unsigned char)(s[i] + k);
}
void rt_xor_and(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n,
                unsigned char k, unsigned char m) {
    for (unsigned long i = 0; i < n; ++i) d[i] = (unsigned char)((s[i] ^ k) & m);
}
void rt_sel(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n,
            unsigned char t, unsigned char lo, unsigned char hi) {
    for (unsigned long i = 0; i < n; ++i) { unsigned char c = s[i]; d[i] = c < t ? lo : hi; }
}
void rt_clamp(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n,
              unsigned char lo, unsigned char hi) {
    for (unsigned long i = 0; i < n; ++i) { unsigned char c = s[i];
        if (c < lo) c = lo; if (c > hi) c = hi; d[i] = c; }
}
void rt_signed(signed char *restrict d, const signed char *restrict s, unsigned long n, signed char k) {
    for (unsigned long i = 0; i < n; ++i) { signed char c = s[i]; d[i] = c < k ? k : c; }
}

/* ---- 3: nested clamps at byte and dword width ------------------------ */
void b_clamp(signed char *restrict d, const signed char *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) { signed char c = s[i];
        if (c < -40) c = -40; if (c > 40) c = 40; d[i] = c; }
}
void i_clamp(int *restrict d, const int *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) { int v = s[i];
        if (v < -40) v = -40; if (v > 40) v = 40; d[i] = v; }
}
void i_clamp3(int *restrict d, const int *restrict s, unsigned long n) {
    /* three levels: the fold must be confluent, not just one level deep */
    for (unsigned long i = 0; i < n; ++i) { int v = s[i];
        if (v < -100) v = -100; if (v > 100) v = 100;
        if (v < -50) v = -50; d[i] = v; }
}

static int fails;
#define CHECK(NAME, DST, REF, ELT, M) \
    do { for (unsigned long q = 0; q < (M); ++q) if ((DST)[q] != (REF)[q]) { \
        printf("FAIL %s n=%lu i=%lu got=%ld want=%ld\n", NAME, (unsigned long)(M), q, \
               (long)(DST)[q], (long)(REF)[q]); if (++fails > 8) exit(1); break; } } while (0)

int main(void) {
    for (int i = 0; i < N; ++i) {
        usrc[i] = (unsigned char)(i < 256 ? i : (i * 167 + 13));
        ssrc[i] = (signed char)usrc[i];
        isrc[i] = (int)(i * 2654435761u) % 301 - 150;
    }
    isrc[0] = -2147483647 - 1; isrc[1] = 2147483647; isrc[2] = -41; isrc[3] = -40;
    isrc[4] = 40; isrc[5] = 41; isrc[6] = 0;

    static const unsigned char ks[] = {0, 1, 32, 127, 128, 200, 254, 255};
    for (unsigned ki = 0; ki < sizeof ks / sizeof ks[0]; ++ki) {
        unsigned char k = ks[ki], m = (unsigned char)(255u - k);
        unsigned char lo = (unsigned char)(k / 2), hi = (unsigned char)(k | 0x80u);
        signed char sk = (signed char)k;
        for (unsigned long n = 0; n <= N; n = (n < 70 ? n + 1 : n * 3 + 1)) {
            unsigned long M = n > N ? N : n;

            memset(udst, 0xAB, sizeof udst);
            for (unsigned long i = 0; i < M; ++i) { volatile unsigned char v = usrc[i]; uref[i] = (unsigned char)(v + k); }
            rt_add(udst, usrc, M, k); CHECK("rt_add", udst, uref, unsigned char, M);

            memset(udst, 0xAB, sizeof udst);
            for (unsigned long i = 0; i < M; ++i) { volatile unsigned char v = usrc[i]; uref[i] = (unsigned char)((v ^ k) & m); }
            rt_xor_and(udst, usrc, M, k, m); CHECK("rt_xor_and", udst, uref, unsigned char, M);

            memset(udst, 0xAB, sizeof udst);
            for (unsigned long i = 0; i < M; ++i) { volatile unsigned char v = usrc[i]; uref[i] = v < k ? lo : hi; }
            rt_sel(udst, usrc, M, k, lo, hi); CHECK("rt_sel", udst, uref, unsigned char, M);

            if (lo <= hi) {
                memset(udst, 0xAB, sizeof udst);
                for (unsigned long i = 0; i < M; ++i) { volatile unsigned char v = usrc[i];
                    unsigned char c = v; if (c < lo) c = lo; if (c > hi) c = hi; uref[i] = c; }
                rt_clamp(udst, usrc, M, lo, hi); CHECK("rt_clamp", udst, uref, unsigned char, M);
            }

            memset(sdst, 0xAB, sizeof sdst);
            for (unsigned long i = 0; i < M; ++i) { volatile signed char v = ssrc[i]; sref[i] = v < sk ? sk : v; }
            rt_signed(sdst, ssrc, M, sk); CHECK("rt_signed", sdst, sref, signed char, M);
        }
    }

    for (unsigned long n = 0; n <= N; n = (n < 70 ? n + 1 : n * 3 + 1)) {
        unsigned long M = n > N ? N : n;

        memset(sdst, 0xAB, sizeof sdst);
        for (unsigned long i = 0; i < M; ++i) { volatile signed char v = ssrc[i];
            signed char c = v; if (c < -40) c = -40; if (c > 40) c = 40; sref[i] = c; }
        b_clamp(sdst, ssrc, M); CHECK("b_clamp", sdst, sref, signed char, M);

        memset(idst, 0xAB, sizeof idst);
        for (unsigned long i = 0; i < M; ++i) { volatile int v = isrc[i];
            int c = v; if (c < -40) c = -40; if (c > 40) c = 40; iref[i] = c; }
        i_clamp(idst, isrc, M); CHECK("i_clamp", idst, iref, int, M);

        memset(idst, 0xAB, sizeof idst);
        for (unsigned long i = 0; i < M; ++i) { volatile int v = isrc[i];
            int c = v; if (c < -100) c = -100; if (c > 100) c = 100; if (c < -50) c = -50; iref[i] = c; }
        i_clamp3(idst, isrc, M); CHECK("i_clamp3", idst, iref, int, M);
    }

    if (fails) { puts("VALIDATION FAILED"); return 1; }
    puts("VALIDATION OK");
    return 0;
}
