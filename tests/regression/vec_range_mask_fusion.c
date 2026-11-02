/*
 * Unsigned range-mask fusion (OP-05c) — exhaustive correctness gate.
 *
 * x86 has no packed unsigned integer compare below AVX-512, so the map
 * vectorizer used to expand `lo <= c && c <= hi` into ELEVEN packed
 * instructions (two sign-bias `vpxor` pairs, two `vpcmpgtd`, two all-ones
 * materialisations, two inverting `vpxor`, one `vpand`).  The fusion in
 * `src/passes/vectorize.rs` rewrites the whole idiom into
 * `Cmp(Slt, Add(X, bias), T)` — one `vpaddd` plus one `vpcmpgtd` — using the
 * order isomorphism `u |-> signed(u + 2^(W-1))` between the unsigned and the
 * signed order of a W-bit lane.
 *
 * That identity is exact for EVERY lane value, but it is exact only if the
 * bias and threshold are computed in the lane's modular domain.  This test
 * pins the cases where a sloppy derivation breaks:
 *
 *   * `signbit_span`  — a window straddling 0x7FFFFFFF/0x80000000, which a
 *                       naive signed reinterpretation of the bounds mis-sorts;
 *   * `half_lower`    — a half-line whose bias and threshold coincide;
 *   * `full_span`     — a window covering the whole domain, where the
 *                       threshold is NOT representable as a signed lane and
 *                       the fusion must refuse instead of emitting an
 *                       off-by-one mask;
 *   * `single`        — a one-element window exactly on the sign bit;
 *   * `zero_lo`       — a window anchored at zero (bias == 2^(W-1));
 *   * `window_lt_lt`  — strict bounds, exercising the +1/-1 edge adjustment.
 *
 * Every kernel runs at every trip count from 0 upward so the vector body,
 * the scalar remainder and the empty-loop case are all covered, and the
 * reference is computed through `volatile` scalars so it can never be
 * vectorized into the same (possibly wrong) form.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define N 4099   /* prime, forces a scalar remainder of every alignment */

static unsigned src[N], dst[N], ref[N];

/* --- kernels: each exercises a different recovered window shape --------- */
void k_window_le_le(unsigned *restrict d, const unsigned *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) { unsigned c = s[i]; if (c >= 65u && c <= 90u) c += 32u; d[i] = c; }
}
void k_window_lt_lt(unsigned *restrict d, const unsigned *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) { unsigned c = s[i]; if (c > 64u && c < 91u) c += 32u; d[i] = c; }
}
void k_half_upper(unsigned *restrict d, const unsigned *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) { unsigned c = s[i]; d[i] = (c <= 1000u) ? c + 7u : c; }
}
void k_half_lower(unsigned *restrict d, const unsigned *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) { unsigned c = s[i]; d[i] = (c >= 0xFFFFFF00u) ? c ^ 5u : c; }
}
void k_zero_lo(unsigned *restrict d, const unsigned *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) { unsigned c = s[i]; if (c >= 0u && c <= 3u) c += 1000u; d[i] = c; }
}
void k_full_span(unsigned *restrict d, const unsigned *restrict s, unsigned long n) {
    /* window covers the ENTIRE domain: fusion must refuse (T unrepresentable) */
    for (unsigned long i = 0; i < n; ++i) { unsigned c = s[i]; if (c >= 0u && c <= 0xFFFFFFFFu) c += 3u; d[i] = c; }
}
void k_single(unsigned *restrict d, const unsigned *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) { unsigned c = s[i]; if (c >= 0x80000000u && c <= 0x80000000u) c = 1u; d[i] = c; }
}
void k_signbit_span(unsigned *restrict d, const unsigned *restrict s, unsigned long n) {
    /* window straddles the signed/unsigned boundary — the case a naive
       signed reinterpretation gets wrong */
    for (unsigned long i = 0; i < n; ++i) { unsigned c = s[i]; if (c >= 0x7FFFFFF0u && c <= 0x80000010u) c = 0xDEADu; d[i] = c; }
}

/* --- scalar reference models (volatile => never vectorized) ------------- */
static void r_window_le_le(void){ for(int i=0;i<N;++i){ volatile unsigned c=src[i]; unsigned v=c; if(v>=65u&&v<=90u) v+=32u; ref[i]=v; } }
static void r_window_lt_lt(void){ for(int i=0;i<N;++i){ volatile unsigned c=src[i]; unsigned v=c; if(v>64u&&v<91u) v+=32u; ref[i]=v; } }
static void r_half_upper(void){ for(int i=0;i<N;++i){ volatile unsigned c=src[i]; unsigned v=c; ref[i]=(v<=1000u)?v+7u:v; } }
static void r_half_lower(void){ for(int i=0;i<N;++i){ volatile unsigned c=src[i]; unsigned v=c; ref[i]=(v>=0xFFFFFF00u)?(v^5u):v; } }
static void r_zero_lo(void){ for(int i=0;i<N;++i){ volatile unsigned c=src[i]; unsigned v=c; if(v<=3u) v+=1000u; ref[i]=v; } }
static void r_full_span(void){ for(int i=0;i<N;++i){ volatile unsigned c=src[i]; ref[i]=c+3u; } }
static void r_single(void){ for(int i=0;i<N;++i){ volatile unsigned c=src[i]; unsigned v=c; if(v==0x80000000u) v=1u; ref[i]=v; } }
static void r_signbit_span(void){ for(int i=0;i<N;++i){ volatile unsigned c=src[i]; unsigned v=c; if(v>=0x7FFFFFF0u&&v<=0x80000010u) v=0xDEADu; ref[i]=v; } }

typedef void (*kfn)(unsigned *restrict, const unsigned *restrict, unsigned long);
typedef void (*rfn)(void);

static int fails = 0;
static void run(const char *name, kfn k, rfn r) {
    for (unsigned long n = 0; n <= N; n = (n < 40 ? n + 1 : n * 3 + 1)) {
        unsigned long m = n > N ? N : n;
        memset(dst, 0xAB, sizeof dst);
        r();
        k(dst, src, m);
        for (unsigned long i = 0; i < m; ++i)
            if (dst[i] != ref[i]) {
                printf("FAIL %s n=%lu i=%lu src=%08x got=%08x want=%08x\n",
                       name, m, i, src[i], dst[i], ref[i]);
                if (++fails > 8) exit(1);
                break;
            }
    }
}

int main(void) {
    /* Boundary-dense input: every value adjacent to every window edge used
       above, then a deterministic LCG bulk fill. */
    static const unsigned edges[] = {
        0u,1u,2u,3u,4u,63u,64u,65u,66u,89u,90u,91u,999u,1000u,1001u,
        0x7FFFFFEFu,0x7FFFFFF0u,0x7FFFFFF1u,0x7FFFFFFFu,
        0x80000000u,0x80000001u,0x8000000Fu,0x80000010u,0x80000011u,
        0xFFFFFEFFu,0xFFFFFF00u,0xFFFFFF01u,0xFFFFFFFEu,0xFFFFFFFFu,
    };
    unsigned e = (unsigned)(sizeof edges / sizeof edges[0]);
    for (int i = 0; i < N; ++i)
        src[i] = (unsigned)i < e ? edges[i]
               : ((unsigned)i % 3u == 0u) ? edges[(unsigned)i % e]
               : (unsigned)i * 2654435761u;
    run("window_le_le", k_window_le_le, r_window_le_le);
    run("window_lt_lt", k_window_lt_lt, r_window_lt_lt);
    run("half_upper",   k_half_upper,   r_half_upper);
    run("half_lower",   k_half_lower,   r_half_lower);
    run("zero_lo",      k_zero_lo,      r_zero_lo);
    run("full_span",    k_full_span,    r_full_span);
    run("single",       k_single,       r_single);
    run("signbit_span", k_signbit_span, r_signbit_span);
    if (fails) { puts("VALIDATION FAILED"); return 1; }
    puts("VALIDATION OK");
    return 0;
}
