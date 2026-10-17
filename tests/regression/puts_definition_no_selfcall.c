/* A TU-defined `puts`: the `fortify_fold` printf->puts rewrite must not turn
 * the `printf` call inside this definition into `call puts@PLT`.  Before A13
 * that is exactly what happened, and the program recursed until SIGSEGV.
 *
 * LCCC-only by construction: every reference compiler measured on the pinned
 * oracles emits the same self-recursive call (GCC 14.2 crashes at -O2, GCC
 * 16.2 / Clang 23.1 / ICX 2025 emit `call puts@PLT` inside `puts`), so no
 * correct oracle exists to compare against.  The lccc self-check still has
 * to pass: it must print `hello` once and exit 0.
 */
#include <stdio.h>

int puts(const char *s) {
    printf("hello\n");
    return s != 0 ? 0 : -1;
}

int main(void) {
    puts("x");
    return 0;
}
