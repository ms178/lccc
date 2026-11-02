#include <stdio.h>
/* Without AVX, `%v` prints nothing and `%d` prints the operand once: the
 * glibc template must become the legacy two-operand `divss %xmm1, %xmm0`.
 * (This TU is the -march=x86-64 build of the modifier probe; the default
 * build of the sibling covers the AVX arm.) */
static float vdiv(float x, float y){ __asm__("%vdivss %1, %d0" : "+x"(x) : "x"(y)); return x; }
static double vsq(double x){ __asm__("%vsqrtsd %d1, %d0" : "=x"(x) : "x"(x)); return x; }
int main(void){ printf("%g %g\n", vdiv(9.0f, 4.0f), vsq(6.25)); return 0; }
