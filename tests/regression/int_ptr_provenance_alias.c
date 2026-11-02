/* An address formed by INTEGER arithmetic carries no provenance from the
 * pointer it was computed from (C23 6.3.2.3, PNVI-ae-udi): with
 * d == (uintptr_t)b - (uintptr_t)a, (char *)((uintptr_t)a + d) designates b.
 * LCCC used to lower the pointer -> integer cast to nothing, so GVN saw
 * `a + d` and rooted the pointer in `a`: f/g kept the cached b[] value
 * across the store through p (printed "1 4"). h pins the store-elimination
 * side: the first store to b[1] is read through p and must survive.
 * Genuine pointer arithmetic (`a + i`) may still be assumed to stay in a. */
#include <stdint.h>
#include <stdio.h>

char a[16], b[16];

__attribute__((noinline)) int f(long d) {
    char *p = (char *)((uintptr_t)a + d);
    b[0] = 1;
    *p = 2;
    return b[0];
}

__attribute__((noinline)) int g(long d, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) {
        char *p = (char *)((uintptr_t)a + d + i);
        b[i] = 1;
        *p = 3;
        s += b[i];
    }
    return s;
}

__attribute__((noinline)) int h(long d) {
    char *p = (char *)((uintptr_t)a + d);
    b[1] = 5; /* read through p below */
    int x = *p;
    b[1] = 9;
    return x;
}

int main(void) {
    long d = (long)((uintptr_t)b - (uintptr_t)a);
    int hr = h(d + 1);
    int fr = f(d);
    int gr = g(d, 4);
    printf("%d %d %d %d\n", fr, gr, hr, b[1]);
    return !(fr == 2 && gr == 12 && hr == 5 && b[1] == 3);
}
