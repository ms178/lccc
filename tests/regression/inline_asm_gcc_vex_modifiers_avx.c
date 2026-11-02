#include <stdio.h>
/* AVX-only shapes of the GCC operand modifiers (see the sibling
 * inline_asm_gcc_vex_modifiers.c): with AVX `%x0, %x0` and `%d0` both
 * spell the destination twice, so the explicit three-operand VEX template
 * and the duplicate-modifier template must agree. */
static float vmul(float x, float y){ __asm__("%vmulss %x1, %x0, %x0" : "+x"(x) : "x"(y)); return x; }
static float vdiv(float x, float y){ __asm__("%vdivss %1, %d0" : "+x"(x) : "x"(y)); return x; }
static double vadd(double x, double y){ __asm__("vaddsd %x1, %x0, %x0" : "+x"(x) : "x"(y)); return x; }
int main(void){ printf("%g %g %g\n", vmul(3.0f, 5.0f), vdiv(9.0f, 4.0f), vadd(1.25, 2.5)); return 0; }
