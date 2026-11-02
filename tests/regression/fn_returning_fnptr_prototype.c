/* Functions RETURNING function pointers must be classified as functions in
 * every declaration context — file-scope prototype, block-scope prototype,
 * and function typedef — never as objects of function-pointer type.
 *
 * Root cause (2026-09-26, found building SQLite 3.53.4's testfixture): the
 * lowering/sema predicate "derived contains Function and no FunctionPointer"
 * treated `void (*f(int))(void);` — derived
 * `[Pointer, FunctionPointer(void), Function(int)]` — as a VARIABLE, so:
 *
 *   - a file-scope prototype emitted a tentative definition `B f` in every TU
 *     (SQLite: "multiple definition of 'sqlite3OsDlSym'"), and for a libc
 *     function it PREEMPTED the real definition: `signal` below resolved to a
 *     zero-filled .bss object and the call jumped into it (SIGSEGV);
 *   - a block-scope prototype became an uninitialised LOCAL function pointer;
 *   - a function typedef returning a function pointer became a
 *     function-POINTER typedef, so `fn_t name;` declared a variable.
 *
 * POSIX spells signal() exactly this way; it is declared here without any
 * header so all three spellings are exercised against the real libc symbol.
 */
#include <stdio.h>

/* 1. file-scope prototype of an external (libc) function */
void (*signal(int, void (*)(int)))(int);
int raise(int);

static volatile int hits;
static void on_usr1(int sig) { hits += sig; }

/* 2. definitions, including a two-level nested return */
static int one(void) { return 1; }
static int two(void) { return 2; }
static long twice(long v) { return 2 * v; }
static long thrice(long v) { return 3 * v; }

int (*pick(int which))(void);
int (*pick(int which))(void) { return which ? two : one; }

long (*(*chooser(int k))(char))(long);
static long (*by_char(char c))(long) { return c == 't' ? thrice : twice; }
long (*(*chooser(int k))(char))(long) { (void)k; return by_char; }

/* 3. function typedef whose return type is a function pointer */
typedef int (*pick_fn_t(int))(void);
pick_fn_t pick; /* redeclaration of the function above, not a variable */

int main(void) {
    const int sigusr1 = 10; /* Linux x86 */
    if (signal(sigusr1, on_usr1) == (void (*)(int))-1)
        return 1;
    raise(sigusr1);
    printf("file-scope signal: hits=%d\n", hits);

    {
        /* 4. block-scope prototype: must bind to the libc function too */
        void (*signal(int, void (*)(int)))(int);
        signal(sigusr1, on_usr1);
        raise(sigusr1);
        printf("block-scope signal: hits=%d\n", hits);
    }

    printf("pick: %d %d\n", pick(0)(), pick(1)());
    printf("chooser: %ld %ld\n", chooser(0)('t')(7), chooser(1)('x')(7));

    pick_fn_t *pp = pick; /* pointer to the function type */
    printf("typedef ptr: %d\n", pp(1)());
    return 0;
}
