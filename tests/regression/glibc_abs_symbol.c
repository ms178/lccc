/* glibc_abs_symbol.c — `.set sym, <int>` ABSOLUTE symbols (glibc
 * localeinfo.h `_NL_CURRENT_DEFINE` emits `.set _nl_current_LC_CTYPE_used, 2`
 * in inline asm). The symbol exists only as a LINK-TIME marker; LCCC used to
 * drop numeric .set targets -> undefined references from setlocale.o at the
 * static libc link. Verifying the link succeeds is the test. */
#include <stdio.h>

extern char _nl_current_LC_CTYPE_used;
extern char _nl_current_LC_TIME_used;

__asm__(".globl _nl_current_LC_CTYPE_used\n.set _nl_current_LC_CTYPE_used, 2");
__asm__(".globl _nl_current_LC_TIME_used\n.set _nl_current_LC_TIME_used, 3");

int main(void) {
    /* Taking addresses forces the ABS relocations into the object; a missing
     * ABS symbol would fail at link time. */
    volatile unsigned long a = (unsigned long)&_nl_current_LC_CTYPE_used;
    volatile unsigned long b = (unsigned long)&_nl_current_LC_TIME_used;
    if (a == 0 || b == 0) { printf("FAIL abs\n"); return 1; }
    printf("PASS abs_symbol\n");
    return 0;
}
