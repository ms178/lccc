/*
 * __LINE__ across line splices (translation phase 2).  Tokens after a
 * backslash-newline are on a LATER physical line than the joined line's
 * start; a single per-output-line start number reports the wrong line.
 */
#include <stdio.h>
#define ADD(a, b) ((a) + (b))
int main(void) {
    int a = 1 + \
            2;
    int spliced_token = \
        __LINE__;
    int after_splice = __LINE__;
    int multi = ADD(1,
                    2) + __LINE__;
    printf("%d %d %d %d\n", a, spliced_token, after_splice, multi);
    return 0;
}
