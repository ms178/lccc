// Regression: the default emit_fused_mul_add (traits.rs) must consume the
// home-less multiply result as the accumulator-side operand of the Add.
// With `acc` loaded first, %eax was clobbered and the mul result (whose only
// location IS %eax — use_count 1, no register, no slot) fell into the i686
// home-less staging fallback that materialises 0: `i*j + 1` returned 1,
// matmul initialised every B[i][j] to the same value at -m32 (all levels).
int printf(const char *, ...);
static double B[8][8];
int main(void) {
    volatile int one = 1;
    int i = one, j = one;
    int a = i * j;        // plain mul (control)
    int b = i * j + 1;    // fused mul+add, constant acc
    int c = 1 + i * j;    // fused mul+add, constant acc (lhs form)
    int e = i * j + j;    // fused mul+add, value acc
    if (a != 1 || b != 2 || c != 2 || e != 2) {
        printf("FAIL a=%d b=%d c=%d e=%d\n", a, b, c, e);
        return 1;
    }
    for (int x = 0; x < 8; x++)
        for (int y = 0; y < 8; y++)
            B[x][y] = (double)(x * y + 1) / 8;
    if (B[7][7] != 50.0 / 8 || B[0][0] != 0.125) {
        printf("FAIL B\n");
        return 1;
    }
    printf("OK\n");
    return 0;
}
