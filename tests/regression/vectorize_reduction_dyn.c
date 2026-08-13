/* Dynamic-loop-bound reduction vectorization: the byte-offset IV computes the
 * vector limit as floor(n/vec_width)*byte_stride at runtime (div+mul), and the
 * scalar remainder starts at byte_iv_final/element_size. A latent dot-product
 * bug deleted the induction-variable increment here (infinite loop). */
#include <stdio.h>
#include <stdlib.h>

double da[1024], db[1024];
float fa[1024], fb[1024];
int ia[1024];

static double dsum(double *x, int n) { double s = 0; for (int i = 0; i < n; i++) s += x[i]; return s; }
static double ddot(double *x, double *y, int n) { double s = 0; for (int i = 0; i < n; i++) s += x[i] * y[i]; return s; }
static float fsum(float *x, int n) { float s = 0; for (int i = 0; i < n; i++) s += x[i]; return s; }
static int isum(int *x, int n) { int s = 0; for (int i = 0; i < n; i++) s += x[i]; return s; }

int main(int argc, char **argv) {
    for (int i = 0; i < 1024; i++) {
        da[i] = (double)(i + 1); db[i] = (double)(i + 1); /* exact in f64 */
        fa[i] = (float)(i + 1); ia[i] = i + 1;
    }
    if (argc > 1) {
        /* single runtime bound from argv */
        int n = atoi(argv[1]);
        long long nn = n;
        if (dsum(da, n) != (double)(nn * (nn + 1)) / 2.0) { printf("FAIL dsum n=%d\n", n); return 1; }
        if (ddot(da, db, n) != (double)(nn * (nn + 1) * (2 * nn + 1)) / 6.0) { printf("FAIL ddot n=%d\n", n); return 1; }
        if (fsum(fa, n) != (float)(nn * (nn + 1)) / 2.0f) { printf("FAIL fsum n=%d\n", n); return 1; }
        if (isum(ia, n) != (int)(nn * (nn + 1)) / 2) { printf("FAIL isum n=%d\n", n); return 1; }
        printf("OK n=%d\n", n);
        return 0;
    }
    /* default: sweep a range of runtime bounds (each is a real runtime value) */
    static const int ns[] = {0, 1, 2, 3, 4, 5, 7, 8, 9, 15, 16, 17, 31, 33, 63, 64, 65, 127, 129, 255, 256, 257, 300};
    for (unsigned k = 0; k < sizeof(ns) / sizeof(ns[0]); k++) {
        int n = ns[k];
        long long nn = n;
        if (dsum(da, n) != (double)(nn * (nn + 1)) / 2.0) { printf("FAIL dsum n=%d\n", n); return 1; }
        if (ddot(da, db, n) != (double)(nn * (nn + 1) * (2 * nn + 1)) / 6.0) { printf("FAIL ddot n=%d\n", n); return 1; }
        if (fsum(fa, n) != (float)(nn * (nn + 1)) / 2.0f) { printf("FAIL fsum n=%d\n", n); return 1; }
        if (isum(ia, n) != (int)(nn * (nn + 1)) / 2) { printf("FAIL isum n=%d\n", n); return 1; }
    }
    printf("OK dynamic bounds sweep\n");
    return 0;
}
