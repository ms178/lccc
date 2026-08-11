/* Regression (v2): reduction vectorization rewired uses of the (now vector)
 * accumulator ONLY in the loop exit block. With two inlined reductions the
 * consumer (here the return-value select) lives in a later block and kept
 * reading the vector accumulator — the backend ADDRESSES vectors (leaq),
 * so the result was a stack address instead of the sum.
 * Failing shape (pre-fix): lea_sib_fold printed -1728274848 instead of 45. */
static int sum(const int *p, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) s += p[i];
    return s;
}

int main(void) {
    int a[9] = {1, 2, 3, 4, 5, 6, 7, 8, 9};
    int b[9] = {9, 8, 7, 6, 5, 4, 3, 2, 1};
    int s1 = sum(a, 9);
    int s2 = sum(b, 9);
    /* consumer in a later block, after both vectorized loops */
    if (s1 != 45 || s2 != 45) return 1;
    if (s1 + s2 != 90) return 2;
    return 0;
}
