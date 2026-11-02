/* glibc's fortified printf family has a fixed prefix (flag/size/format)
 * followed by `...`; everything past the prefix must receive the C11
 * 6.5.2.2p6 default argument promotions.  glibc's <stdio.h> rewrites
 * `printf("%g", f)` into `__builtin___printf_chk(1, "%g", f)`; without the
 * promotions a float arrived as a 4-byte F32 and was read as a double. */
#include <stdio.h>

int main(void)
{
    float f = 1.25f;
    char c = -3;
    unsigned short us = 62000;
    __builtin___printf_chk(1, "%g %d %u\n", f, c, us);
    char buf[64];
    int n = __builtin___snprintf_chk(buf, sizeof buf, 1, sizeof buf,
                                     "%g %d\n", 2.75f, (short)-9);
    printf("n=%d %s", n, buf);
    return 0;
}
