/* Guard test (v2): address-of-extern and global access must work under the
 * default PIC code model (GOTPCREL for external symbols). Exercises the
 * needs_got_for_addr path that the PIC-default change made the common case,
 * including a call through a function pointer stored in a global. */
#include <stdio.h>

int g_val = 7;
static int (*g_fn)(int) = 0;

static int add7(int x) { return x + g_val; }

int main(void) {
    g_fn = add7;
    if ((*g_fn)(35) != 42) { printf("FAIL fnptr\n"); return 1; }
    if (g_val != 7) { printf("FAIL global\n"); return 2; }
    printf("PASS pic_extern_addr\n");
    return 0;
}
