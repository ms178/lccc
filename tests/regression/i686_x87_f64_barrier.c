/* LCCC advertises declared-type evaluation, including its i686 x87 backend.
   A non-popping FST rounds memory but not the forwarded x87 register. */
#include <float.h>
static volatile double a=0x1.0000002p0, b=0x1.ffffffcp-1, c=-1.0;
__attribute__((noinline)) double cancel(double x, double y, double z) {
    return x*y+z;
}
int main(void) {
    if (FLT_EVAL_METHOD != 0) return 2;
    return cancel(a,b,c) != 0.0;
}
