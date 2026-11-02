/* Scalar sqrt must match libm within tight tolerance (VEX encoding path). */
#include <math.h>
int main(void) {
    volatile double x = 4.0;
    double y = sqrt(x);
    if (y < 1.999999 || y > 2.000001) return 1;
    x = 2.0;
    y = sqrt(x);
    if (y < 1.414213 || y > 1.414214) return 2;
    x = 0.0;
    y = sqrt(x);
    if (y != 0.0) return 3;
    return 0;
}
