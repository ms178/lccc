/* SysV AMD64 aggregate register assignment is transactional. Five integer
 * parameters leave one GP register; a two-eightbyte INTEGER struct cannot fit
 * and goes wholly to the stack, but the following scalar must still use r9.
 * va_start must then begin immediately after the stacked struct. */
#include <stdarg.h>
#include <stdlib.h>

struct pair { long first, second; };

__attribute__((noinline))
static void check(int a, int b, int c, int d, int e,
                  struct pair stacked, int last_named, ...)
{
    va_list ap;
    va_start(ap, last_named);
    int anonymous = va_arg(ap, int);
    va_end(ap);
    if (a || b || c || d || e)
        abort();
    if (stacked.first != 11 || stacked.second != 22)
        abort();
    if (last_named != 33 || anonymous != 44)
        abort();
}

int main(void)
{
    struct pair value = {11, 22};
    check(0, 0, 0, 0, 0, value, 33, 44);
    return 0;
}
