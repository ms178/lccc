/* Regression (v2): the driver rejected `-m64` ("unsupported machine option"),
 * breaking zlib-ng's configure which passes -m64 on x86-64. -m64 is the
 * default target; it must be accepted as a no-op like -m32 selects i686.
 * (flags file adds -m64.) */
#include <stdio.h>
int main(void) {
    if (sizeof(void *) != 8) { printf("FAIL not 64-bit\n"); return 1; }
    printf("PASS m64\n");
    return 0;
}
