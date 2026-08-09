#include <math.h>
#include <complex.h>
static volatile int calls;
static int side(int x) { ++calls; return x; }
int main(void) {
    int x = 3, *p = &x;
    calls = 0;
    if (!(x == 3 && side(1))) return 1;
    if (calls != 1) return 2;
    calls = 0;
    if (0 && side(1)) return 3;
    if (calls != 0) return 4;
    if (side(0) && 0) return 5;
    if (calls != 1) return 6;
    calls = 0;
    if (!(side(1) || 1)) return 7;
    if (calls != 1) return 8;
    if (!p || (p == 0)) return 9;
    if (-0.0) return 10;
    if (!NAN) return 11;
    double _Complex z = 0.0 + 2.0 * I;
    if (!z) return 12;
    if ((x > 0 && p) ? 0 : 1) return 13;
    return 0;
}
