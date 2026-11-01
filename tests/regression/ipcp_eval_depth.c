/* Exercise IPCP with a recursively non-linear function. The positive control
 * must fold, while the much deeper constant call remains executable at run
 * time instead of exhausting the compiler's evaluation stack. */
#include <stdio.h>

__attribute__((noinline)) static int chain(int n) {
    if (n == 0) return 0;
    return (chain(n - 1) * 3 + n) & 0xffff;
}

int main(int argc, char **argv) {
    (void)argv;
    volatile int shallow = chain(1000);
    printf("%d\n", shallow);
    if (argc > 64) {
        volatile int deep = chain(400000);
        printf("%d\n", deep);
    }
    return 0;
}
