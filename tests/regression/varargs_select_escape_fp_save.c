/* A va_list passed through a conditional pointer must retain both GP and SSE
 * register-save areas.  At -O2 if-conversion rewrites the source-level
 * `consume ? &ap : 0` phi to Select; escape analysis must follow that alias.
 *
 * The former x86 prologue scan followed Copy only, concluded that no local
 * va_arg used an FP value, and omitted every XMM save.  The forwarded double
 * was then read from uninitialized stack storage. */
#include <stdarg.h>
#include <stdlib.h>

__attribute__((noinline))
static void consume_args(va_list *ap)
{
    if (!ap)
        abort();
    if (va_arg(*ap, int) != 17)
        abort();
    if (va_arg(*ap, double) != 0.625)
        abort();
}

__attribute__((noinline))
static void forward_args(int consume, ...)
{
    va_list ap;
    va_start(ap, consume);
    consume_args(consume ? &ap : (void *)0);
    va_end(ap);
}

int main(void)
{
    forward_args(1, 17, 0.625);
    return 0;
}
