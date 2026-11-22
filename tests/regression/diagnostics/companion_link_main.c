/* COMPANION harness, program half: links companion_link_body.c (named in the
 * .env sidecar) into the same executable. Both lccc and the GCC oracle build
 * the pair, and their stdout+exit code must be identical. The result is also
 * checked against the closed form 13 * 3 + 1 = 40, so a wrong companion object
 * fails even when both compilers agree on the wrong answer. */
#include <stdio.h>

int companion_link_body_entry(int x);

int main(void)
{
    int r = companion_link_body_entry(13);
    printf("companion=%d\n", r);
    return r == 40 ? 0 : 1;
}
