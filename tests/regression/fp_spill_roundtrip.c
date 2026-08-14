// Regression: a float-heavy accumulation loop whose intermediate F64 values
// spill to stack slots. The peephole must fold the XMM->GPR->stack round-trip
// (`movq %xmmN,%rax; movq %rax,off(%rbp)`) into one `movsd %xmmN,off(%rbp)`
// without changing the result.
#include <stdio.h>

static double kernel(double *a, double *b, int n) {
    double acc = 0.0;
    for (int i = 0; i < n; i++) {
        double p = a[i] * 0.5 + b[i] * 0.25;
        double q = p * p + 1.5;
        double r = q - a[i] * b[i];
        acc += r * 0.125;
    }
    return acc;
}

int main(void) {
    double a[256], b[256];
    for (int i = 0; i < 256; i++) { a[i] = i * 0.1; b[i] = (255 - i) * 0.2; }
    printf("fp_spill total: %.4f\n", kernel(a, b, 256));
    return 0;
}
