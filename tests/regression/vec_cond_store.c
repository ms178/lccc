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
 *
 * References use volatile scalars so they cannot share the transform.
 */
#include <stdio.h>

#define N 1031

static unsigned char p[N], q[N], ref[N];

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
    }
    k_volatile_store(p, N);
    if (fails == 0) printf("ALL OK\n");
    return fails != 0;
}
