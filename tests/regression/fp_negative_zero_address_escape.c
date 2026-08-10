/* Regression: an address-taken stack source must survive peephole FP shuttling. */
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

int main(void) {
    volatile double source = -0.0;
    int exponent = 0;
    double x = source;
    double y = frexp(x, &exponent);
    uint64_t x_bits, y_bits;
    memcpy(&x_bits, &x, sizeof x_bits);
    memcpy(&y_bits, &y, sizeof y_bits);
    printf("%016llx %016llx %d\n", (unsigned long long)x_bits,
           (unsigned long long)y_bits, exponent);
    return x_bits != y_bits || exponent != 0;
}
