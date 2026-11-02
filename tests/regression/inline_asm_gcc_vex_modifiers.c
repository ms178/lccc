#include <stdio.h>
/* GCC x86 operand modifiers that switch on the TU's AVX availability,
 * oracle-checked against gcc 16.2 (glibc's math-inline-asm.h relies on
 * every one of them):
 *   %v   -> `v` prefix with AVX, nothing without;
 *   %dN  -> operand printed twice ("dst, dst") with AVX, once without,
 *           so `%vdivss %1, %d0` yields the 3-operand VEX form or the
 *           2-operand legacy form — never the mixed `vdivss %1, %0`
 *           that GAS rejects ("number of operands mismatch");
 *   %xN / %tN / %gN -> the xmm / ymm / zmm spelling of a vector operand.
 * `%d` on a GPR operand prints the natural width twice. */
static float vdiv(float x, float y){ __asm__("%vdivss %1, %d0" : "+x"(x) : "x"(y)); return x; }
static float vsub(float x, float y){ __asm__("%vsubss %x1, %d0" : "+x"(x) : "x"(y)); return x; }
static unsigned mx(void){ unsigned m; __asm__ volatile("%vstmxcsr %0" : "=m"(m)); return m; }
static long twice(long a){ long r; __asm__("leaq (%1,%1), %0" : "=r"(r) : "r"(a)); return r; }
int main(void){
    printf("%g %g %d %ld\n", vdiv(9.0f, 4.0f), vsub(3.0f, 5.0f), (mx() & 0x1f80) == 0x1f80, twice(21));
    return 0;
}
