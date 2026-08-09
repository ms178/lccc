/* glibc_vstmxcsr.c — %vstmxcsr / %vldmxcsr (VEX-encoded MXCSR ops, glibc
 * math fclrexcpt.c uses VPREFIX "%v" + stmxcsr). Also covers the %v / %x
 * GNU-as mnemonic hints and the 2-operand AVX scalar form
 * (`%vdivss %1, %d0` with the GCC 'd' duplicate-operand modifier). */
#include <stdio.h>

static unsigned int __attribute__((noinline)) get_mxcsr(void) {
    unsigned int m;
    __asm__ volatile ("%vstmxcsr %0" : "=m"(m));
    return m;
}

static void __attribute__((noinline)) set_mxcsr(unsigned int m) {
    __asm__ volatile ("%vldmxcsr %0" : : "m"(m));
}

static float __attribute__((noinline)) div_f(float x, float y) {
    __asm__ volatile ("%vdivss %1, %d0" : "+x"(x) : "x"(y));
    return x;
}

int main(void) {
    unsigned int m = get_mxcsr();
    set_mxcsr(m);                     /* round-trip */
    if (get_mxcsr() != m) { printf("FAIL vstmxcsr\n"); return 1; }
    if (div_f(6.0f, 2.0f) != 3.0f) { printf("FAIL vdivss\n"); return 1; }
    printf("PASS vstmxcsr\n");
    return 0;
}
