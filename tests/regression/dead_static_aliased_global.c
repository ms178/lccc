/* A file-local object becomes observable under a linker alias. Dead-static
 * store elimination must not use only GlobalAddr("hidden") to conclude that
 * its stores are unobservable: another TU can read either exported name. */
#include <stdio.h>

static int hidden_strong;
static int hidden_weak;
extern int exported __attribute__((alias("hidden_strong")));
extern int weak_export __attribute__((weak, alias("hidden_weak")));

__attribute__((noinline)) void write_hidden(int a, int b) {
    hidden_strong = a;
    hidden_weak = b;
}

int main(void) {
    write_hidden(123, 456);
    printf("%d %d\n", exported, weak_export);
    return exported != 123 || weak_export != 456;
}
