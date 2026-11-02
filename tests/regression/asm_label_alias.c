/* __asm__("label") linker redirects + alias/weak attributes
 * (the v1 glibc-configure bug class). Verifies the symbols resolve and the
 * aliased function is callable under both names. */
#include <stdio.h>

/* Redirect the definition to a different linker symbol. */
extern int hidden_real(int x) __asm__("zzz_real_fn");
int hidden_real(int x) { return x * 3; }

/* Alias: foo_alias -> bar (weak). */
extern int bar_fn(int x);
int bar_fn(int x) { return x + 7; }
extern __typeof(bar_fn) foo_alias __attribute__((weak, alias("bar_fn")));

int main(void) {
    if (hidden_real(4) != 12) return 1;
    if (bar_fn(3) != 10) return 2;
    if (foo_alias(3) != 10) return 3;
    /* The asm label must be the actual symbol (checked at link time: if the
     * compiler emitted "hidden_real" instead of "zzz_real_fn", the strong
     * definition may collide with the extern decl and link differently). */
    printf("OK asm_label_alias\n");
    return 0;
}
