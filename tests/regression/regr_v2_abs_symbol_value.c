/* Regression (v2): numeric `.set sym, <number>` top-level asm symbols
 * (glibc _NL_CURRENT_DEFINE style) are ABSOLUTE symbols: their address IS
 * their value (ELF spec). Under the PIC default they must be emitted as
 * `movq $sym` — a GOTPCREL reference cannot be resolved for an absolute
 * symbol (original bug: "undefined symbols: 4660, 7" at link;
 * glibc_abs_symbol.c covers the nonzero form).
 *
 * This test asserts the exact ELF values (7 and 0x1234). NOTE: GCC 16.1.1
 * on this host cannot pass it — its ld rebases absolute-symbol references
 * by the load base even for non-PIE executables (proven reduced-case
 * baseline defect: a=4198498 for value 7 with -Wl,-no-pie). LCCC emits the
 * ELF-correct `movq $sym`. */
#include <stdio.h>

extern char _nl_test_abs_1;
extern char _nl_test_abs_2;

__asm__(".globl _nl_test_abs_1\n.set _nl_test_abs_1, 7");
__asm__(".globl _nl_test_abs_2\n.set _nl_test_abs_2, 0x1234");

int main(void) {
    volatile unsigned long a = (unsigned long)&_nl_test_abs_1;
    volatile unsigned long b = (unsigned long)&_nl_test_abs_2;
    if (a != 7) { printf("FAIL a=%lu\n", a); return 1; }
    if (b != 0x1234) { printf("FAIL b=%lu\n", b); return 2; }
    printf("PASS abs_symbol_value\n");
    return 0;
}
