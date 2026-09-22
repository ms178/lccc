/* Undefined weak symbol referenced from executable code (glibc/zstd
 * zstd_trace.h tracing-hook shape): the address compare `(fn != NULL)`
 * materialises the symbol address. With a direct PC32 LEA the system
 * linker rejects the default PIE link ("relocation R_X86_64_PC32 against
 * undefined symbol ... can not be used when making a PIE object"); the
 * fix routes weak addresses through @GOTPCREL, which resolves in every
 * code model. Pre-fix this test fails to LINK under the default driver —
 * the regression IS the link failure. Differential vs GCC. */
#include <stdio.h>

extern int exotic_trace_hook(void) __attribute__((weak));

int main(void) {
    int v = 41;
    if (exotic_trace_hook != NULL) {
        v += exotic_trace_hook();
    }
    printf("v=%d\n", v);
    return v == 41 ? 0 : 1;
}
