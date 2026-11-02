/* globaldse_unread_static.c — TU dead-global-store elimination.

   A static output buffer written in a loop and never read. The stores are
   unobservable within the TU, so lccc's Phase 11a may delete them (mirroring
   GCC/Clang whole-program DSE). The program's observable behavior must stay
   identical: it prints a constant, not the buffer contents.

   This pins that the transform is sound for the unread case and that the
   buffer's removal does not affect surrounding code.
*/
#include <stdio.h>

static double out[4096];

__attribute__((noinline)) void fill_out(void) {
    for (int i = 0; i < 4096; i++) {
        out[i] = (double)i * 0.5;
    }
}

int main(void) {
    fill_out();
    /* out is never read — only this constant is observable */
    printf("PASS globaldse_unread_static %d\n", 42);
    return 0;
}
