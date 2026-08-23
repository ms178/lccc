/* OP-12/OP-32: store-to-load forwarding across deliberately site-local
 * GlobalAddr duplicates.
 *
 * g1[i] = 5; g2[i] = 6; return g1[i];   — the second g1[i] load must be
 * forwarded to the stored constant 5 (the two GlobalAddr instructions for
 * g1 carry different value numbers because OP-34 keeps variable-index
 * bases site-local; the canonical address key must unify them).
 * g2's intervening store must NOT block the forward (distinct object).
 * The plain (may-alias) pointer form must still reload. */
int g1[100], g2[100];

__attribute__((noinline)) int fwd(int i) {
    g1[i] = 5;
    g2[i] = 6;
    return g1[i];
}

/* Same shape through unrestrictable pointer params: the store through p
 * may alias q, so the second q[i] load MUST survive. */
__attribute__((noinline)) int no_fwd(int i, int *p, int *q) {
    q[i] = 5;
    p[i] = 6;
    return q[i];
}

int main(void) {
    if (fwd(3) != 5) return 1;
    if (g1[3] != 5 || g2[3] != 6) return 2;

    /* p and q alias the same array: the second load must observe 6. */
    int a[4] = {0, 0, 0, 0};
    if (no_fwd(1, a, a) != 6) return 3;

    /* Disjoint p and q: the second load observes 5. */
    int b[4] = {0, 0, 0, 0};
    int c[4] = {0, 0, 0, 0};
    if (no_fwd(2, b, c) != 5) return 4;
    return 0;
}
