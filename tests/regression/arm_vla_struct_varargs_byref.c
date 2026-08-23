/* AArch64 AAPCS64: variable-size aggregate variadic arguments are passed by
 * reference. va_arg(ap, typeof(vla_struct)) must read the pointer and copy the
 * runtime byte count. Reduced from gcc.c-torture/execute/20020412-1.c.
 */
extern void abort(void);
#include <stdarg.h>

__attribute__((noinline)) void check(int n, ...) {
    struct V { char x[n]; } v;
    va_list ap;
    va_start(ap, n);
    v = va_arg(ap, typeof(v));
    for (int i = 0; i < n; ++i)
        if (v.x[i] != '0' + i) abort();
    v = va_arg(ap, typeof(v));
    for (int i = 0; i < n; ++i)
        if (v.x[i] != '5' + i) abort();
    va_end(ap);
}

int main(void) {
    int n = 5;
    struct X { char x[n]; } a, b;
    for (int i = 0; i < n; ++i) {
        a.x[i] = '0' + i;
        b.x[i] = '5' + i;
    }
    check(n, a, b);
    return 0;
}
