/* glibc_f128_casts.c — _Float128 <-> i128 conversions (glibc wcstof128_l.c
 * casts F128 -> unsigned __int128; LCCC used to panic "unsupported
 * float-to-i128 conversion: F128"). Via libgcc __fixtfti/__fixunstfti/
 * __floattitf/__floatuntitf. */
#include <stdio.h>

int main(void) {
    _Float128 f = 1.0f128;
    __int128 i = (__int128)f;
    unsigned __int128 u = (unsigned __int128)(f * 42.0f128);
    _Float128 back = (_Float128)i;
    if (i != 1) { printf("FAIL f128->i128\n"); return 1; }
    if (u != 42) { printf("FAIL f128->u128\n"); return 1; }
    if (back != 1.0f128) { printf("FAIL i128->f128\n"); return 1; }
    printf("PASS f128_casts\n");
    return 0;
}
