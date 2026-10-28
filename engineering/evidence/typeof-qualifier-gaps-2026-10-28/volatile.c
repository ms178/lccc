/* GCC 16.2 treats target as volatile; LCCC currently does not. */
volatile int source;
__typeof__(source) target;

int read_twice(void) {
    return target + target;
}

void discard_twice(void) {
    target;
    target;
}
