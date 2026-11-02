/* va_list can escape through static storage without being a call argument.
 * The variadic prologue must then save both GP and SSE argument registers for
 * a later callee that consumes the global list. */
#include <stdarg.h>
#include <stdlib.h>

static va_list shared;

__attribute__((noinline))
static void consume(void)
{
    if (va_arg(shared, int) != 23)
        abort();
    if (va_arg(shared, double) != 0.75)
        abort();
}

__attribute__((noinline))
static void publish(int tag, ...)
{
    va_start(shared, tag);
    consume();
    va_end(shared);
}

int main(void)
{
    publish(0, 23, 0.75);
    return 0;
}
