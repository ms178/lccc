/* Integer lane algebra only; the four FP additions retain source order.
 * poly's signed wrap is intentional: this fixture requires -fwrapv. */
__attribute__((noinline))
double packed_poly(const double *values, int n, int bias) {
    double sum = 0.0;
    for (int j = 0; j < n; ++j) {
        int denominator = ((j + bias) * (j + bias + 1)) / 2 + j + 1;
        sum += (1.0 / (double)denominator) * values[j];
    }
    return sum;
}
__attribute__((noinline))
double packed_bits(const double *values, int n, int bias) {
    double sum = 0.0;
    int b = bias & 32767;
    for (int j = 0; j < n; ++j) {
        int t = j + b;
        int denominator = (((t * 4) ^ (t >> 3)) & 0x3fffffff) + 1;
        sum += (1.0 / (double)denominator) * values[j];
    }
    return sum;
}
/* A variable shift deliberately retains scalar-DAG packing. */
__attribute__((noinline))
double scalar_shift(const double *values, int n, int bias) {
    double sum = 0.0;
    for (int j = 0; j < n; ++j) {
        int denominator = 1 << ((j + (bias & 7)) & 7);
        sum += (1.0 / (double)denominator) * values[j];
    }
    return sum;
}
