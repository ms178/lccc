/*
 * __LINE__ inside a macro body expands to the line of the INVOCATION
 * (C11 6.10.8), including when the invocation spans several physical
 * lines: the pin is the line of the macro name token, not the line where
 * argument collection happens to end.
 */
#include <stdio.h>
#define HERE() (__LINE__)
#define HERE_OBJ (__LINE__)
#define WRAP(x) (x)
#define WHERE() (__LINE__)
#define CHAIN WHERE()
#define SUM2(a, b) ((a) + (b))
int main(void) {
    int a = HERE();
    int b = HERE_OBJ;
    int c = WRAP(HERE());
    int d = SUM2(1,
                 2) + HERE();
    int e = CHAIN;
    printf("%d %d %d %d %d\n", a, b, c, d, e);
    return 0;
}
