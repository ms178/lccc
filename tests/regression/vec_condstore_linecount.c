/* Conditional store with a data-dependent counter inside the if-body
 * (zstd programs/util.c UTIL_processLines shape): the vectorizer's
 * conditional-store rewrite replaced the body with a predicated
 * select-store and silently dropped the `lineCount++` increment, so the
 * function returned 0 for every single-line file and `zstd --filelist`
 * failed with "error reading <file>". The rewrite must fail closed when
 * a loop-carried phi is fed from a rewritten block. Differential vs GCC. */
#include <stdio.h>

__attribute__((noinline)) static size_t processLines(char *b, size_t n) {
    size_t lineCount = 0;
    for (size_t i = 0; i < n; i++) {
        if (b[i] == '\n') {
            b[i] = '\0';
            lineCount++;
        }
    }
    if (n > 0 && (n == 0 || b[n - 1] != '\0'))
        lineCount++;
    return lineCount;
}

int main(void) {
    char buf[32];
    for (int k = 0; k < 2; k++) {
        for (int i = 0; i < 31; i++)
            buf[i] = (char)('a' + (i % 26));
        buf[30] = '\n';
        buf[31] = '\0';
        size_t lines = processLines(buf, 31);
        printf("k=%d lines=%zu first=%c\n", k, lines, buf[0]);
        if (lines != 1) {
            printf("FAIL: expected 1 line\n");
            return 1;
        }
    }
    return 0;
}
