/* gcc.c-torture/execute/20041114-1.c reduced.
 * On the false edge of `var <= 0`, signed int var is positive, so
 * `(unsigned)(var - 1) < UINT_MAX` is necessarily true without invoking signed
 * overflow. The link_failure edge must be deleted at -O2.
 */
#include <limits.h>
void link_failure(void);
volatile int v;
void foo(int var) {
    if (!(var <= 0 || ((long unsigned)(unsigned)(var - 1) < UINT_MAX)))
        link_failure();
}
int main(void) {
    foo(v);
    return 0;
}
