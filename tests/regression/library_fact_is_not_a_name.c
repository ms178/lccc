/* What the compiler knows about a C library function belongs to the calls
 * that reach the library, not to every call that spells the name.
 *
 * -ffreestanding leaves `strlen` an ordinary identifier, and this program
 * defines its own: it counts how often it runs.  `__builtin_strlen` is still
 * the library function, by the programmer's own choice of spelling.  Using it
 * once anywhere in the translation unit used to mark every plain `strlen(s)`
 * read-only as well, so the two calls in `twice` were merged into one and the
 * counter came up short (-O2 and above; Clang 23.1.5 and GCC 16.2 call twice).
 */
#include <stdio.h>

typedef __SIZE_TYPE__ size_t;

static int calls;
static volatile int zero;

/* No loop: nothing here is a `strlen` idiom for a compiler to recognise. */
__attribute__((noinline)) size_t strlen(const char *s)
{
    calls++;
    return !s[0] ? 0 : !s[1] ? 1 : !s[2] ? 2 : !s[3] ? 3 : 4;
}

__attribute__((noinline)) static size_t twice(const char *s)
{
    size_t first = strlen(s);
    size_t second = strlen(s);
    return first + second;
}

__attribute__((noinline)) static size_t discarded(const char *s)
{
    strlen(s);
    return 1;
}

__attribute__((noinline)) static size_t library(const char *s)
{
    return __builtin_strlen(s);
}

int main(void)
{
    static char text[] = "four";
    const char *s = text + zero;
    size_t length = library(s);
    int before = calls;
    size_t sum = twice(s);
    int in_twice = calls - before;
    before = calls;
    sum += discarded(s);
    int in_discarded = calls - before;

    printf("length %zu, sum %zu, calls %d and %d\n", length, sum, in_twice, in_discarded);
    return !(length == 4 && sum == 9 && in_twice == 2 && in_discarded == 1);
}
