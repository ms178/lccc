/*
 * #line overrides compose with physical-line resolution: after `#line N`
 * the next physical line is N, and offsets advance by PHYSICAL lines
 * (comment-collapse and splices shift the mapping, not the override).
 */
#include <stdio.h>
int main(void) {
    int before = __LINE__;
#line 100
    int first = __LINE__;
    int second = __LINE__;
    /* comment
       spanning lines */ int after_comment = __LINE__;
    int continued = 1 + \
        2, after_splice = __LINE__;
#line 500
    int restarted = __LINE__;
    printf("%d %d %d %d %d %d\n", before, first, second, after_comment,
           after_splice, restarted);
    return 0;
}
