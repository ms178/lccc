/* Salvaged from levkropp/lccc a9987d0; strengthened with executable checks. */
/* Regression: cmp+select fusion (backend/generation.rs detect_cmp_select_fusion)
 * used to move the flag-setting compare from the Cmp's IR position to the
 * Select's, with no adjacency requirement. Intervening instructions could
 * legally recycle the compare operand's physical register, so the fused
 * `cmp reg, #0` tested a stale value — here the (a^b)&0x80 condition is
 * clobbered by the intervening cast/and feeding the inner select, and the
 * outer select always picks the ±1 arm instead of preserving `res`.
 * sqlite's vdbeSorterCompareInt mis-sorted records exactly like this.
 * Correct output: -7 7 1 0
 */
extern int printf(const char *, ...);

__attribute__((noinline)) int cmp_bytes(const unsigned char *p1, const unsigned char *p2, int n) {
    int res = 0;
    for (int i = 0; i < n; i++) {
        if ((res = p1[i] - p2[i]) != 0) {
            if (((p1[0] ^ p2[0]) & 0x80) != 0) {
                res = (p1[0] & 0x80) ? -1 : +1;
            }
            break;
        }
    }
    return res;
}

int main(void) {
    unsigned char a[2] = {8, 15};
    unsigned char b[2] = {15, 8};
    unsigned char c[2] = {8, 8};
    printf("%d %d %d %d\n",
           cmp_bytes(a, b, 2), cmp_bytes(b, a, 2),
           cmp_bytes(a, b, 2) != 0, cmp_bytes(c, c, 2));
    /* Independent scalar oracle, all byte pairs and a second-byte mismatch.
     * Runtime data and noinline prevent constant folding the interesting body. */
    for (unsigned i = 0; i < 256; i++) {
        for (unsigned j = 0; j < 256; j++) {
            unsigned char x[2] = {(unsigned char)i, 15};
            unsigned char y[2] = {(unsigned char)j, 8};
            int want = i == j ? 7 : (i < 128) != (j < 128) ? (i < 128 ? 1 : -1) : (int)i - (int)j;
            if (cmp_bytes(x, y, 2) != want || cmp_bytes(x, x, 2) != 0 ||
                cmp_bytes(x, y, 0) != 0) return 1;
        }
    }
    return 0;
}
