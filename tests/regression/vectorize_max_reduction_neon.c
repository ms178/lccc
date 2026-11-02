// NEON i32 max reduction (levkropp 8b139820, audited port): find_max-shaped
// loops (Select-form after if_convert, marching-pointer access) vectorize to
// smax/smaxv via the late AArch64 pass. Edge matrix: n below/at/above vector
// width, remainder counts, iv_init=1 (shifted coverage) vs 0, both compare
// polarities, all-negative arrays (smax lane semantics), INT_MIN seed.
// x86 keeps the scalar loop (NEON-only gate) and must print identically.
int printf(const char *, ...);
static int a[4096];
int fm(int n) { int m = a[0]; for (int i = 1; i < n; i++) { int x = a[i]; if (x > m) m = x; } return m; }
int fm0(int n) { int m = -2147483647 - 1; for (int i = 0; i < n; i++) { int x = a[i]; if (x > m) m = x; } return m; }
int fm2(int n) { int m = a[0]; for (int i = 1; i < n; i++) { int x = a[i]; if (m < x) m = x; } return m; }
int main() {
    for (int i = 0; i < 4096; i++) a[i] = (int)((i * 2654435761u) % 100000) - 50000;
    a[0] = -99999;
    int ns[] = {1, 2, 3, 4, 5, 7, 8, 9, 15, 16, 17, 100, 3999, 4000, 4096};
    for (unsigned k = 0; k < sizeof(ns) / sizeof(ns[0]); k++)
        printf("%d:%d,%d,%d ", ns[k], fm(ns[k]), fm0(ns[k]), fm2(ns[k]));
    printf("\n");
    for (int i = 0; i < 4096; i++) a[i] = -100000 - i;
    printf("neg:%d,%d\n", fm(4000), fm0(4000));
    return 0;
}
