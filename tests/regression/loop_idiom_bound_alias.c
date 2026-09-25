/* The header loads N on every iteration. The byte store may change N via a
 * plain unsigned-char pointer, so neither hoisting the bound nor replacing
 * the loop with a single memmove is legal. Initially N=3; writing S[0]=2
 * shortens the original loop before it reaches S[2]=17. */
#include <stdio.h>

static unsigned n = 3;
static unsigned char source[4] = {2, 0, 17, 9};

__attribute__((noinline)) static void copy_to(unsigned char *dst) {
    for (unsigned i = 0; i < n; ++i) dst[i] = source[i];
}

int main(void) {
    copy_to((unsigned char *)&n);
    printf("loop-idiom bound alias: %u\n", n);
    return n != 2;
}
