/* expat xml_name_continue / ctype-style classification, in the BOOLEAN-
 * PREDICATE form real parsers use: a small `int f(char)` returning 0/1 that
 * the caller sums or branches on.
 *
 * This shape is what drives range_fold -> set_membership: the `&&`/`||` chain
 * if-converts to a Select chain, each `lo <= c && c <= hi` folds to one
 * unsigned range test, and the [a-z]/[A-Z] pair then merges into a single
 * test on `c & ~32`. The counting form (`if (pred) n++;`) does NOT produce
 * Selects and stays a compare/branch chain -- k_namechars.c covers that case,
 * so the two kernels together bracket the classifier problem.
 */
#include <stddef.h>
#include "bench.h"

#define N 4096
static char s[N];

void bench_setup(void) {
    const char *alpha = "abcXYZ019_-.<>&\" \t\n";
    for (int i = 0; i < N; i++) {
        s[i] = alpha[i % 19];
    }
}

static int is_name_char(char ch) {
    return ((ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') ||
            (ch >= '0' && ch <= '9') || ch == '_' || ch == '-' || ch == '.');
}

unsigned long long bench_run(void) {
    unsigned long long c = 0;
    for (int i = 0; i < N; i++) {
        c += (unsigned) is_name_char(s[i]);
    }
    return c;
}
