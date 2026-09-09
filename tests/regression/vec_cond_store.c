/*
 * Conditional-store statement form (PR #455 port with audit fixes) — gate.
 *
 * The rewrite turns `L=load p[i]; if (c1 && c2) store f(L) to p[i]` into a
 * nested-Select map shape against the dominating same-address load.  This
 * file pins:
 *   (a) the statement-form casefold — must vectorize or run scalar, but be
 *       byte-exact either way, for every trip count;
 *   (b) the audit's MISCOMPILE repro: load `p[4-i]`, store `p[i-4]` — the
 *       swapped-Sub "same address" proof must NEVER fire (wrong-address
 *       stores);
 *   (c) a cast in the second condition block (the dangling-SSA shape) —
 *       decline is fine, wrong code is not;
 *   (d) a volatile stream — never rewritten.
 *   (e) the IN-PLACE statement form (`c = p[i]; if (c1 && c2) p[i] = f(c)`)
 *       — the same-address load dominates the guarded store, so the
 *       rewrite applies and the loop must vectorize or run scalar,
 *       byte-exact either way, for every trip count.
 *   (f) the in-place single-compare dword form (the map parser's dword
 *       conditional path).
 *
 * References use volatile scalars so they cannot share the transform.
 */
#include <stdio.h>

#define N 1031

static unsigned char p[N], q[N], ref[N];
static int iq[N], iref[N];

static void k_casefold(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        unsigned char c = s[i];
        if (c >= 'A' && c <= 'Z') d[i] = (unsigned char)(c + 32);
    }
}
static void k_swap_sub(unsigned char *restrict d, unsigned long n) {
    /* In-bounds different-address shape: the load GEP offset is
     * Sub(n-1, i) — a Sub-derived offset that is NOT the store's
     * i-4.  The same-address proof must reject (the swap arm of
     * `operand_equiv` accepted Sub(a,b)~Sub(b,a) pre-fix); the
     * rewrite then declines and the scalar loop must stay exact. */
    for (unsigned long i = 4; i < n; ++i) {
        unsigned char L = d[(n - 1) - i];
        if (L) d[i - 4] = (unsigned char)(L + 1);
    }
}
static void k_cast_chain(unsigned char *restrict d, const unsigned char *restrict s, int flag, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        unsigned char c = s[i];
        if (c >= 'A' && (char)flag == 5) d[i] = (unsigned char)(c + 32);
    }
}
static volatile unsigned char vsink[16];
static void k_volatile_store(const unsigned char *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        unsigned char c = s[i];
        if (c == 0x2A) vsink[i & 15] = c;
    }
}
/* (e) in-place byte form: load p[i] dominates the guarded store to p[i]. */
static void k_inplace_casefold(unsigned char *restrict p, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        unsigned char c = p[i];
        if (c >= 'A' && c <= 'Z') p[i] = (unsigned char)(c + 32);
    }
}
/* (f) in-place dword form, single compare. */
static void k_inplace_min(int *restrict p, int lo, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        int v = p[i];
        if (v < lo) p[i] = lo;
    }
}
/* (g) in-place or-mask with a CONSTANT stored value. */
static void k_inplace_ormask(unsigned char *restrict p, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        unsigned char c = p[i];
        if (c == 0 || c > 250) p[i] = 0xFF;
    }
}
/* (h) in-place three-condition chain: folded mask + residual compare. */
static void k_inplace_three(unsigned char *restrict p, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        unsigned char c = p[i];
        if (c >= 'a' && c <= 'z' && c != 'q') p[i] = (unsigned char)(c - 32);
    }
}

/* volatile references */
static void v_casefold(unsigned char *restrict d, const unsigned char *restrict s, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        volatile unsigned char c = s[i];
        if (c >= 'A' && c <= 'Z') d[i] = (unsigned char)(c + 32);
    }
}
static void v_swap_sub(unsigned char *restrict d, unsigned long n) {
    for (unsigned long i = 4; i < n; ++i) {
        volatile unsigned char L = d[(n - 1) - i];
        if (L) d[i - 4] = (unsigned char)(L + 1);
    }
}
static void v_cast_chain(unsigned char *restrict d, const unsigned char *restrict s, int flag, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        volatile unsigned char c = s[i];
        if (c >= 'A' && (char)flag == 5) d[i] = (unsigned char)(c + 32);
    }
}
static void v_inplace_casefold(unsigned char *restrict p, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        volatile unsigned char c = p[i];
        if (c >= 'A' && c <= 'Z') p[i] = (unsigned char)(c + 32);
    }
}
static void v_inplace_min(int *restrict p, int lo, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        volatile int v = p[i];
        if (v < lo) p[i] = lo;
    }
}
static void v_inplace_ormask(unsigned char *restrict p, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        volatile unsigned char c = p[i];
        if (c == 0 || c > 250) p[i] = 0xFF;
    }
}
static void v_inplace_three(unsigned char *restrict p, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        volatile unsigned char c = p[i];
        if (c >= 'a' && c <= 'z' && c != 'q') p[i] = (unsigned char)(c - 32);
    }
}

int main(void) {
    for (int i = 0; i < 256 && i < N; i++) p[i] = (unsigned char)i;
    for (int i = 256; i < N; i++) p[i] = (unsigned char)(i * 131 + 17);
    int fails = 0;
#define CHECK(a, b, name) do { if ((a) != (b)) { printf("FAIL %s\n", name); fails++; } } while (0)
    /* Curated trip counts: empty, 1, near-boundary, VF multiples
     * (16/32/64), the VF+1 forms, and the prime N.  Every element is
     * still compared byte-exactly at every count. */
    static const unsigned long counts[] = {0, 1, 2, 15, 16, 17, 31, 32, 33, 47, 64, 65, 96, 97, N};
    for (unsigned ci = 0; ci < sizeof(counts) / sizeof(counts[0]); ci++) {
        unsigned long n = counts[ci];
        for (int i = 0; i < N; i++) { q[i] = p[i]; ref[i] = p[i]; }
        k_casefold(q, p, n); v_casefold(ref, p, n);
        for (int i = 0; i < N; i++) CHECK(q[i], ref[i], "casefold");
        for (int i = 0; i < N; i++) { q[i] = p[i]; ref[i] = p[i]; }
        k_swap_sub(q, n); v_swap_sub(ref, n);
        for (int i = 0; i < N; i++) CHECK(q[i], ref[i], "swap_sub");
        for (int i = 0; i < N; i++) { q[i] = p[i]; ref[i] = p[i]; }
        k_cast_chain(q, p, 5, n); v_cast_chain(ref, p, 5, n);
        for (int i = 0; i < N; i++) CHECK(q[i], ref[i], "cast_chain");
        for (int i = 0; i < N; i++) { q[i] = p[i]; ref[i] = p[i]; }
        k_inplace_casefold(q, n); v_inplace_casefold(ref, n);
        for (int i = 0; i < N; i++) CHECK(q[i], ref[i], "inplace_casefold");
        for (int i = 0; i < N; i++) { iref[i] = iq[i] = (int)i * 7 - 300; }
        k_inplace_min(iq, -100, n); v_inplace_min(iref, -100, n);
        for (int i = 0; i < N; i++) CHECK(iq[i], iref[i], "inplace_min");
        for (int i = 0; i < N; i++) { q[i] = p[i]; ref[i] = p[i]; }
        k_inplace_ormask(q, n); v_inplace_ormask(ref, n);
        for (int i = 0; i < N; i++) CHECK(q[i], ref[i], "inplace_ormask");
        for (int i = 0; i < N; i++) { q[i] = p[i]; ref[i] = p[i]; }
        k_inplace_three(q, n); v_inplace_three(ref, n);
        for (int i = 0; i < N; i++) CHECK(q[i], ref[i], "inplace_three");
    }
    k_volatile_store(p, N);
    if (fails == 0) printf("ALL OK\n");
    return fails != 0;
}
