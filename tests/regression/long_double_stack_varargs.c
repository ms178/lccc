/* Regression: SysV x86-64 long double varargs are 16-byte stack arguments. */
#include <stdio.h>
#include <string.h>

int main(void) {
    char buffer[64];
    int n = sprintf(buffer, "%Lg %d", (long double)1.5, 33);
    printf("n=%d b=%s\n", n, buffer);
    return n != 6 || strcmp(buffer, "1.5 33") != 0;
}
