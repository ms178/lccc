/* MachInst regression: signed negative GEP indices must be sign-extended.
 * SQLite FTS5 fts5PorterStep1B2 computes n-2; n==1 previously became
 * +0xffffffff and dereferenced wild memory. */
#include <stdio.h>
__attribute__((noinline)) static int pick(const unsigned char *base, volatile int *n) {
    int index = *n - 2;
    return base[index];
}
int main(void) {
    unsigned char bytes[5] = {7,11,22,33,44};
    volatile int n = 1;
    int got = pick(&bytes[2], &n);
    if (got != 11) { printf("FAIL got=%d expected=11\n", got); return 1; }
    puts("OK machinst_signed_gep");
    return 0;
}
