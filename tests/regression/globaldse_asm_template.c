/* globaldse_asm_template.c — inline-asm template mention must keep stores (F5).

   A store whose only reader is `asm("movl g(%%rip), %0" : "=r"(v))` must be
   kept. The old Phase 11a only checked input_symbols, not the template
   string, so it would delete the store. This test reads `g` via inline asm
   and checks the value.

   Note: the same gap existed in the older dead-static-function pass; both
   passes now scan template strings with word-boundary matching.
*/
#include <stdio.h>

static int g;

__attribute__((noinline)) void set_g(void) {
    g = 5;
}

__attribute__((noinline)) int read_g_via_asm(void) {
    int v;
    __asm__ volatile("movl g(%%rip), %0" : "=r"(v));
    return v;
}

int main(void) {
    set_g();
    int v = read_g_via_asm();
    if (v != 5) {
        printf("FAIL asm_template got %d want 5\n", v);
        return 1;
    }
    printf("PASS globaldse_asm_template %d\n", v);
    return 0;
}
