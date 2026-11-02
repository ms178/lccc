/* VLA-containing structs in variadic calls are passed by reference by GCC on
 * the supported ABIs. va_arg(ap, typeof(d)) fetches that pointer; consumers copy
 * the runtime size from the referenced object. Exercises two va_arg fetches so
 * va_list advancement is covered too (gcc.c-torture/execute/20020412-1.c).
 */
#include <stdarg.h>
extern void abort(void);

__attribute__((noinline)) void foo(int size, ...) {
    struct { char x[size]; } d;
    va_list ap;
    va_start(ap, size);
    d = va_arg(ap, typeof(d));
    for (int i = 0; i < size; ++i)
        if (d.x[i] != '0' + i) abort();
    d = va_arg(ap, typeof(d));
    for (int i = 0; i < size; ++i)
        if (d.x[i] != '5' + i) abort();
    va_end(ap);
}

int main(void) {
    int z = 5;
    struct { char a[z]; } x, y;
    for (int i = 0; i < z; ++i) {
        x.a[i] = '0' + i;
        y.a[i] = '5' + i;
    }
    foo(z, x, y);
    return 0;
}
