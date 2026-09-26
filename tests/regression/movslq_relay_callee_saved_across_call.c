/* A callee-saved register survives a call: a relay fold must not treat the
 * call as the end of its lifetime.
 *
 * Root cause (2026-09-26, found reducing SQLite's wherePathSolver): the
 * peephole windowed dead-register scan (`is_reg_dead_after`, also its
 * relaxed sibling) answered "dead" for EVERY register the moment it reached
 * a `call`, reasoning that calls clobber caller-saved registers. The
 * widened size below is homed in callee-saved %rbx, so
 *
 *     movslq %eax, %rbx ; movq %rbx, %rdi ; call malloc ; ... movq %rbx, %rdx
 *
 * was folded to `movslq %eax, %rdi`, deleting the only write of %rbx, and
 * memset received whatever %rbx held on entry as its length (SIGSEGV at
 * -O2). A call is now transparent to callee-saved families, kills only the
 * caller-saved families it does not read, and keeps the argument registers
 * it consumes (LCCC_CALL_ARGS) live.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

__attribute__((noinline)) static char *make(int n, int mx) {
    int sz = (16 + 8 * n) * mx * 2 + 64;
    char *p = malloc(sz); /* small: never NULL in practice */
    memset(p, 0x5a, sz);
    return p;
}

/* Same shape with the widened value needed after two calls. */
__attribute__((noinline)) static long twice(int n) {
    long w = (long)(n * 3 + 1);
    char *a = malloc(w);
    char *b = malloc(w);
    long r = w;
    if (a && b) {
        memset(a, 1, w);
        memcpy(b, a, w);
        r += b[w - 1];
    }
    free(a);
    free(b);
    return r;
}

int main(void) {
    int n = 3, mx = 10;
    int sz = (16 + 8 * n) * mx * 2 + 64;
    char *p = make(n, mx);
    int ok = p != 0;
    for (int i = 0; ok && i < sz; i++)
        ok &= p[i] == 0x5a;
    free(p);
    printf("%d %ld\n", ok, twice(n));
    return 0;
}
