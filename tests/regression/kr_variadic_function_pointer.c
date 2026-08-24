/* K&R parameter declarations must preserve a pointed-to function's variadic
 * tail through parser metadata, semantic call checking, and IR call ABI. */
#include <stdarg.h>
#include <stdlib.h>

typedef int (*variadic_fn)(int, ...);

static int sum3(int first, ...)
{
    va_list ap;
    int result;
    va_start(ap, first);
    result = first + va_arg(ap, int) + va_arg(ap, int);
    va_end(ap);
    return result;
}

static int invoke(fp)
    int (*fp)(int, ...);
{
    return (*fp)(10, 20, 30);
}

int main(void)
{
    variadic_fn modern = sum3;
    if (invoke(modern) != 60)
        abort();
    return 0;
}
