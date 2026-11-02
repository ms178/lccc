#include <stdarg.h>

/* eliminate_fp_spill_around_load must not keep the pending va_arg double in
 * %xmm0 across the pxor/cvtsi2sd pair before the later reload.  That pair is
 * before K, not in the old (K,L)-only scan; the value must be relayed through
 * %xmm1 (or spilled), otherwise the final addition doubles 144 and main
 * returns 1.  Reduced from gcc.c-torture/execute/stdarg-4.c at -O0. */
long x;

inline void __attribute__((always_inline)) consume(va_list ap)
{
    x = va_arg(ap, double);
    x += va_arg(ap, long);
    x += va_arg(ap, double);
}

void f(int i, ...)
{
    va_list ap;
    va_start(ap, i);
    consume(ap);
    va_end(ap);
}

int main(void)
{
    f(3, 16.0, 128L, 32.0);
    return x != 176L;
}
