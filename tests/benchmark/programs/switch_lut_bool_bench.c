#include <stdio.h>

#ifndef N
#define N 8000000
#endif

__attribute__((noinline)) static int is_vowel(signed char c) {
    switch (c) {
    case 'a': case 'e': case 'i': case 'o': case 'u':
    case 'A': case 'E': case 'I': case 'O': case 'U':
        return 1;
    default:
        return 0;
    }
}

int main(void) {
    unsigned seed = 5;
    long sum = 0;
    for (unsigned i = 0; i < N; ++i) {
        seed = seed * 1664525u + 1013904223u;
        sum += is_vowel((signed char)(seed >> 24));
    }
    printf("switch_lut_bool: %ld\n", sum);
    return 0;
}
