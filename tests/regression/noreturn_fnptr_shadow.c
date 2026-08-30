/* noreturn name-shadowing: a function-pointer parameter named like a
 * __noreturn function must NOT inherit the attribute.
 *
 * The boot decompressor's `void (*error)(char *x)` parameter shadowing
 * error.h's `void error(char *) __noreturn` made every call through the
 * pointer terminate with Unreachable: ud2 after the indirect call. The
 * userspace preboot ZSTD oracle SIGILL'd; the preboot kernel lost its
 * post-error() control flow. */
#include <stdio.h>
#include <stdlib.h>

void error(char *m) __attribute__((noreturn));
void error(char *m) { printf("global error: %s\n", m); exit(9); }

static void cb(char *m) { printf("cb: %s\n", m); }

__attribute__((noinline))
static int h(size_t code, void (*error)(char *x))
{
    if (code == 0)
        return 0;
    switch (code) {
    case 1: error((char *)"one"); break;
    case 2: error((char *)"two"); break;
    default: error((char *)"many"); break;
    }
    return -1; /* reachable when the pointer returns */
}

int main(void)
{
    printf("h(2, cb) = %d\n", h(2, cb));
    return 0;
}
