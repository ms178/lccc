/* strcmp/memcmp of string literals answer with the SIGN, which is all C
 * specifies and what gcc 14.2 and clang 23.1.3 both fold to; glibc's runtime
 * answer is a magnitude, so folding to the magnitude made the same program
 * print different numbers at -O0 and -O2.
 *
 * Baseline on main: the literal rows printed glibc magnitudes at every level
 * (`memcmp("c","a",1)` -> 2, `memcmp("z","a",1)` -> 25, `strcmp("abc","abx")`
 * -> -21) where both reference compilers print 1 / 1 / -1.
 *
 * Rows are either LITERAL (both operands are string literals, so the fold must
 * answer with the sign, and gcc's fold is the oracle) or OPAQUE (one operand is
 * selected at runtime through a `volatile` selector, so no compiler can fold and
 * glibc answers with its own magnitude -- identical for lccc and gcc). The
 * opaque rows exist to prove the fix did not turn into a blanket rewrite of
 * every compare call, and they assert SIGN CLASSES only: asserting an exact
 * runtime magnitude would be over-specified against one vendor (measured:
 * clang 23.1.3 at -O2 returns 256 for a runtime `strcmp(a,"b")` where gcc and
 * lccc return 1 -- both conforming, because only the sign is specified).
 *
 * A `static` pointer object with a constant initializer is NOT a usable way to
 * make a row opaque: gcc propagates it, emits `call *strlen(%rip)`, which binds
 * libc's versioned symbol and fails to link as PIE. Hence the volatile selector.
 *
 * -O0 keeps every call (that tier's contract is "skip the optimizer
 * completely"), so at -O0 the literal rows print glibc's magnitudes exactly like
 * gcc -O0 does; this file therefore asserts signs only for the folded levels and
 * sign classes everywhere. Do not add an -O0 sidecar fold to make this file
 * print signs at -O0: gcc does not.
 */
#include <stdio.h>
#include <string.h>

volatile int pick_first = 0;

static int sign(int v) { return (v > 0) - (v < 0); }

int main(void)
{
    /* LITERAL rows: the fold must produce exactly these values, and gcc's
     * folded output is byte-identical. */
    printf("memcmp c/a=%d a/c=%d z/a=%d hi/lo=%d\n",
           memcmp("c", "a", 1), memcmp("a", "c", 1),
           memcmp("z", "a", 1), memcmp("\x80", "\x7f", 1));
    printf("memcmp prefixes=%d equal=%d\n",
           memcmp("ab", "aa", 2), memcmp("abc", "abc", 3));
    printf("strcmp c/a=%d abc/abx=%d equal=%d\n",
           strcmp("c", "a"), strcmp("abc", "abx"), strcmp("abc", "abc"));
    printf("strlen=%d %d\n", (int)strlen("abc"), (int)__builtin_strlen(""));

    /* OPAQUE rows: sign classes only, glibc answers at every level. */
    const char *left = pick_first ? "z" : "a";
    printf("opaque signs=%d %d %d\n",
           sign(memcmp(left, "m", 1)),
           sign(strcmp(left, "m")),
           sign(memcmp("m", left, 1)));

    /* A TU-visible declaration of the name is not shadowed here, so the compare
     * contract also has to hold when the operands reach the fold through a
     * helper's parameter (inlining at -O2 turns them back into literals). */
    printf("done\n");
    return 0;
}
