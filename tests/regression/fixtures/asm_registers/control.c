/* Negative control of check_asm_registers.py: the gate compiles this file
   for lccc with -DPLANTED, so the two programs print different numbers by
   construction, and the gate has to report it. */
#include <stdio.h>

int main(void) {
#ifdef PLANTED
    puts("control 2");
#else
    puts("control 1");
#endif
    return 0;
}
