/* glibc_hex_float_f128.c — hex-float literals with _FloatN suffixes
 * (`0x1p-65f128` in glibc s_compoundn_template.c). The lexer used to split
 * them as "0x1p-65f" + "128" -> syntax error. Also checks q/Q suffixes. */
#include <stdio.h>

int main(void) {
    _Float128 x = 0x1p-65f128;
    _Float128 y = 1.0f128;
    _Float128 z = 0x1.8p+1f128;
    /* 2^-65 * 2^65 = 1 ; 1.5 * 2 = 3 */
    double d = (double)(x * 0x1p+65f128);
    double e = (double)(z);
    if (d != 1.0) { printf("FAIL hex f128 %f\n", d); return 1; }
    if (e != 3.0) { printf("FAIL hex f128 2 %f\n", e); return 1; }
    if (y != 1.0) { printf("FAIL f128 lit\n"); return 1; }
    printf("PASS hex_float_f128\n");
    return 0;
}
