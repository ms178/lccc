/*
 * __LINE__ after a multi-line block comment.  The comment's internal
 * newlines disappear in translation phase 3, so the output line holding
 * `tail` contains bytes of TWO physical lines; C11 requires __LINE__ to
 * expand to the physical line of the token, i.e. the line of the closing
 * comment end for tokens following the comment on the same output line.
 */
#include <stdio.h>
int main(void) {
    /* a block comment
       that spans
       three physical lines */
    int after = __LINE__;
    int x = 1; /* tail comment
                  spanning lines */ int tail = __LINE__;
    int simple = __LINE__;
    printf("%d %d %d\n", after, tail, simple);
    return 0;
}
