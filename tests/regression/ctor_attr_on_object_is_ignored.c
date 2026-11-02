/* `constructor`/`destructor` name a FUNCTION the loader calls.
 *
 * Original defect: lccc honoured the attribute on any declaration, so
 *   void (*fp)(void) __attribute__((constructor)) = init;
 * emitted `.init_array: .quad fp` -- the ADDRESS OF A DATA OBJECT -- which
 * the dynamic loader then called before main, killing the process with
 * SIGSEGV. GCC ignores the attribute on a non-function (with
 * `'constructor' attribute ignored`) and the program runs normally.
 *
 * A declaration that is not defined in this translation unit must not leave
 * an entry either (the entry belongs to the defining TU), while a genuine
 * constructor -- including one whose attribute is written on an earlier
 * prototype -- must still run before main, and a destructor after it.
 */
#include <stdio.h>

static int ran;

static void init(void) { ran = 1; }
static void fini(void) { printf("fini ran=%d\n", ran); }

/* Ignored by GCC: an object of function-pointer type, not a function. */
void (*fp)(void) __attribute__((constructor)) = init;

static void proto_ctor(void) __attribute__((constructor));
static void proto_ctor(void) { ran = 1; }

static void proto_dtor(void) __attribute__((destructor));
static void proto_dtor(void) { fini(); }

int main(void) {
    if (fp != init) {
        printf("FAIL fp points at %p\n", (void *)fp);
        return 1;
    }
    if (ran != 1) {
        printf("FAIL constructor did not run\n");
        return 2;
    }
    printf("PASS ctor_attr_on_object_is_ignored\n");
    return 0;
}
