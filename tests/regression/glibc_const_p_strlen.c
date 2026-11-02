/* glibc_const_p_strlen.c — `__builtin_constant_p(__builtin_strlen(msg))`
 * with a string literal must fold to 1 after inlining (glibc startup.h
 * `_startup_fatal`). LCCC used to leave the not-constant fallback call in
 * the object -> undefined `_startup_fatal_not_constant` at static link. */
#include <stdio.h>

static int not_constant_triggered = 0;

static void startup_fatal(const char *message) {
    size_t l = __builtin_strlen(message);
    if (!__builtin_constant_p(l)) {
        not_constant_triggered = 1;  /* must be dead code */
    }
    (void)l;
}

int main(void) {
    startup_fatal("Fatal glibc error: Cannot allocate TLS block\n");
    if (not_constant_triggered) { printf("FAIL const_p\n"); return 1; }
    printf("PASS const_p_strlen\n");
    return 0;
}
