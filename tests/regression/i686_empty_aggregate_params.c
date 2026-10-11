/* GNU zero-sized aggregates consume neither argument bytes nor registers.
   Keep each parameter object addressable so optimization cannot hide a missing
   callee classification arm. Also usable as a mixed-compiler ABI fixture:
   compile bodies with -DCALLEE_ONLY and main with -DCALLER_ONLY. */
#include <stdarg.h>

struct empty { char bytes[0]; };
_Static_assert(sizeof(struct empty) == 0, "GNU zero-sized aggregate");

__attribute__((noinline)) int mixed(unsigned, struct empty, double, int);
__attribute__((noinline, regparm(3))) int registers(unsigned, struct empty, unsigned, unsigned);
__attribute__((noinline, fastcall)) int fast(unsigned, struct empty, unsigned, unsigned);
__attribute__((noinline)) int variable(unsigned, struct empty, ...);
__attribute__((noinline, fastcall)) int fast_first(struct empty, unsigned, struct empty, unsigned, unsigned);

#ifndef CALLER_ONLY
__attribute__((noinline)) int mixed(unsigned a, struct empty hole, double x, int b)
{
    __asm__ volatile ("" : : "r" (&hole) : "memory");
    return (int) a * 17 + b + (int) x;
}

__attribute__((noinline, regparm(3))) int registers(unsigned a, struct empty hole, unsigned b, unsigned c)
{
    __asm__ volatile ("" : : "r" (&hole) : "memory");
    return (int) (a * 17 + b * 3 + c);
}

__attribute__((noinline, fastcall)) int fast(unsigned a, struct empty hole, unsigned b, unsigned c)
{
    __asm__ volatile ("" : : "r" (&hole) : "memory");
    return (int) (a * 17 + b * 3 + c);
}

__attribute__((noinline, fastcall)) int fast_first(struct empty first, unsigned a, struct empty middle, unsigned b, unsigned c)
{
    __asm__ volatile ("" : : "r" (&first), "r" (&middle) : "memory");
    return (int) (a * 17 + b * 3 + c);
}

__attribute__((noinline)) int variable(unsigned marker, struct empty hole, ...)
{
    va_list ap;
    int x, y;
    __asm__ volatile ("" : : "r" (&hole) : "memory");
    va_start(ap, hole);
    x = va_arg(ap, int);
    y = va_arg(ap, int);
    va_end(ap);
    return (int) marker + 3 * x + y;
}
#endif

#ifndef CALLEE_ONLY
static volatile unsigned input = 7;
int main(void)
{
    struct empty e;
    unsigned a = input;
    if (mixed(a, e, 8.5, 13) != 140) return 1;
    if (registers(a, e, 13, 5) != 163) return 2;
    if (fast(a, e, 13, 5) != 163) return 3;
    if (variable(a, e, 13, 17) != 63) return 4;
    if (fast_first(e, a, e, 13, 5) != 163) return 5;
    return 0;
}
#endif
