#include <stdio.h>
static int hidden;
__attribute__((noinline)) void set_hidden(int v) { hidden = v; }
int main(void) {
    int observed;
    set_hidden(123);
    __asm__ volatile ("movl hidden(%%rip), %0" : "=r"(observed) : : "memory");
    printf("%d\n", observed);
    return 0;
}
