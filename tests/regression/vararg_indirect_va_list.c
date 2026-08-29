/* Variadic register-save escape analysis: a va_list whose address arrives
 * through memory (an array of va_list pointers — every textual mention
 * re-loads the pointer under a fresh SSA id) must conservatively force the
 * register-save prologue. The dead-save elimination used to see no escape
 * (the callee read uninitialized save-area bytes: "hello (null)"), fixed by
 * the fail-closed root-stability guard (CCC_NO_VA_ROOT_GUARD reverts it).
 * gcc.c-torture va-arg-21/va-arg-13/stdarg-2 shapes. */
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>

static void __attribute__((__format__(__printf__, 1, 2)))
doit(const char *s, ...)
{
    va_list *ap_array[3], **ap_ptr = ap_array;

    ap_array[0] = malloc(sizeof(va_list));
    ap_array[1] = NULL;
    ap_array[2] = malloc(sizeof(va_list));

    va_start (*ap_array[0], s);
    vprintf (s, **ap_ptr);
    /* Side effect in the va_end argument: the pointer increment must run. */
    va_end (**ap_ptr++);

    ap_ptr++;

    va_start (*ap_array[2], s);
    /* If ap_ptr was not advanced twice, this dereferences NULL. */
    vprintf (s, **ap_ptr);
    va_end (**ap_ptr);

    if (*ap_ptr == 0)
        abort();
    free(ap_array[0]);
    free(ap_array[2]);
}

/* Heap-allocated va_list read back through a re-load (va-arg-13 shape). */
static int sum_via_ptr(int n, ...)
{
    va_list *p = malloc(sizeof(va_list));
    int r = 0;
    va_start (*p, n);
    for (int i = 0; i < n; i++)
        r += va_arg (*p, int);
    va_end (*p);
    free(p);
    return r;
}

/* Integer-only wrapper keeps the XMM dead-save elimination (a stable local
 * root must still skip the 8 XMM stores — this pins the optimization the
 * guard must NOT damage). */
static int sum_int(int n, ...)
{
    va_list ap;
    va_start(ap, n);
    int s = 0;
    for (int i = 0; i < n; i++)
        s += va_arg(ap, int);
    va_end(ap);
    return s;
}

int main(void)
{
    doit("%s world\n", "hello");
    if (sum_via_ptr(3, 10, 20, 30) != 60) { puts("sum_via_ptr FAIL"); return 1; }
    if (sum_int(4, 1, 2, 3, 4) != 10) { puts("sum_int FAIL"); return 1; }
    puts("indirect va_list PASS");
    return 0;
}
