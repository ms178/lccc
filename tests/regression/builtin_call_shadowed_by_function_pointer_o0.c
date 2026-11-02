/* The same shadowing class at -O0 (see the .flags file), where the defect was a
 * crash rather than a wrong number: the lowerer emitted `call strlen` against a
 * file-scope `size_t (*)(const char *)` OBJECT, the assembler bound the reference
 * to that object's address, and the program executed its own data -- SIGSEGV,
 * exit 139, where gcc 14.2 exits 0 and prints 103.
 *
 * Kept separate from the -O2 test because the two levels fail differently: at
 * -O2 the constant fold produced a wrong value, at -O0 (no folding) the call
 * target was wrong. A single test at one level would have hidden the other.
 */
#include <stdio.h>

static size_t my_strlen(const char *s) { size_t n = 0; while (s[n]) ++n; return 100 + n; }
static int my_abs(int v) { return 70 + (v < 0 ? -v : v); }

/* internal linkage: see the -O2 test for the measured glibc-versioned-symbol
 * hazard that an external-linkage object of this name runs into */
static size_t (*strlen)(const char *) = my_strlen;
static int (*abs)(int) = my_abs;

int main(void)
{
    printf("strlen=%zu\n", strlen("abc"));
    printf("abs=%d\n", abs(-5));
    printf("builtin=%d\n", (int)__builtin_strlen("abcd"));
    return 0;
}
