/* expat XML name scanning: multi-range byte classification.
 * Exercises range folding, set-membership recognition and if-conversion. */
#include <stddef.h>
#include "bench.h"

#define N 4096
static char s[N];

void bench_setup(void) {
    const char *alpha = "abcXYZ019_-.<>&\" \t\n";
    for (int i = 0; i < N; i++) s[i] = alpha[i % 19];
}

static size_t count_name_chars(const char *p, size_t n) {
    size_t c = 0;
    for (size_t i = 0; i < n; i++) {
        char ch = p[i];
        if ((ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') ||
            (ch >= '0' && ch <= '9') || ch == '_' || ch == '-' || ch == '.')
            c++;
    }
    return c;
}

unsigned long long bench_run(void) { return count_name_chars(s, N); }
