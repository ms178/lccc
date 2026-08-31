/*
 * Preprocessor line-resolver: a file that has BOTH a backslash-newline
 * splice (join_map is Some) AND a multi-line block comment (comment_map
 * is Some) must still expand __LINE__ to the PHYSICAL source line.
 *
 * This is the sqlite3.c amalgamation shape in miniature. A prior
 * build_line_resolver cloned the whole join_map into a fresh Rc on every
 * output line (Θ(N²)); correctness of the shared-Rc path is pinned here,
 * scale is pinned by the lib test
 * build_line_resolver_is_linear_on_large_spliced_commented_file.
 *
 * Physical lines of interest (1-based, after this comment block):
 *   splice opens on the `int a = 1 + \` line
 *   `__LINE__` after the multi-line comment must match gcc.
 */
#include <stdio.h>
int main(void) {
    int a = 1 + \
        2;
    /* multi
       line comment */
    int after = __LINE__;
    int b = 3 + \
        4; int same = __LINE__;
    printf("%d %d %d\n", after, same, a + b);
    return 0;
}
