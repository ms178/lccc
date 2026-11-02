/*
 * PF-15 chain form: the frontend represents strcmp's loop as a signed-byte
 * load, a widening cast for the loop condition, then a second widening cast
 * (separated from the first by the other load) for the equality comparison.
 *
 * Every first byte is tested exhaustively.  This includes negative `char`
 * values on the x86 target and the two early-termination cases, so an invalid
 * signed/unsigned predicate rewrite or a lost zero-test is observable.
 */
#include <stdio.h>

int strcmp_byte_chain(const char *a, const char *b) {
    while (*a && *a == *b) {
        ++a;
        ++b;
    }
    return (int)(unsigned char)*a - (int)(unsigned char)*b;
}

int main(void) {
    char a[2];
    char b[2];
    int failures = 0;

    a[1] = 0;
    b[1] = 0;
    for (int ai = 0; ai != 256; ++ai) {
        for (int bi = 0; bi != 256; ++bi) {
            a[0] = (char)ai;
            b[0] = (char)bi;
            int got = strcmp_byte_chain(a, b);
            int want = ai - bi;
            if (got != want) {
                ++failures;
            }
        }
    }

    if (failures != 0) {
        printf("widened-byte-chain FAIL %d\n", failures);
        return 1;
    }
    puts("widened-byte-chain OK");
    return 0;
}
