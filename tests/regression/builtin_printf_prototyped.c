/* __builtin_printf family with <stdio.h> present: the declared libc
 * prototype IS variadic, and the trailing args beyond the fixed prefix must
 * still receive the C11 6.5.2.2p6 default argument promotions. */
#include <stdio.h>

int main(void)
{
    float f = 2.5f;
    char c = -7;
    unsigned short us = 61000;
    __builtin_printf("%g %d %u\n", f, c, us);
    /* The plain call path must keep promoting identically. */
    printf("%g %d %u\n", f, c, us);
    char buf[64];
    int n = __builtin_snprintf(buf, sizeof buf, "%g %u\n", 3.25f, us);
    printf("n=%d %s", n, buf);
    return 0;
}
