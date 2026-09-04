/* __builtin_printf / __builtin_snprintf in a TU with NO <stdio.h>: the
 * variadic contract of the printf family belongs to the builtin's own
 * signature, not to a user-declared libc prototype.  C11 6.5.2.2p6 default
 * argument promotions must still apply: float -> double, char/short (any
 * signedness) -> int.  A float passed as a 4-byte F32 in an XMM register
 * was read by glibc's printf as a double (garbage); a char/short passed in
 * a narrowed register was read as int (stale upper bits). */
int main(void)
{
    float f = 1.5f;
    float g = 0.1f;
    char c = -1;
    signed char sc = -2;
    unsigned char uc = 250;
    short s = -30000;
    unsigned short us = 60000;
    __builtin_printf("%g %d %d %u %d %u %d\n", f, c, sc, uc, s, us, 42);
    __builtin_printf("%.17g\n", g);
    __builtin_printf("%d %d\n", (_Bool)5, (_Bool)0);
    return 0;
}
