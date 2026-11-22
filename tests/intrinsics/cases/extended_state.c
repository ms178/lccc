// FLAGS: -mxsave
/* XGETBV: the register state the operating system keeps.
 *
 * `_xgetbv(0)` reads XCR0. Runtime dispatch asks it whether AVX (bits 1 and
 * 2) and AVX-512 (bits 5 to 7) may be used: CPUID says the processor has
 * them, XCR0 says the system saves them. It was a stub returning 0, so
 * every program that asked ran its slowest path. Bit 0 (x87) is set on
 * every system, so a zero is never the answer; the value itself is the
 * machine's, and GCC's program reads the same register. */
#include <immintrin.h>
#include <stdio.h>

__attribute__((noinline)) static long long control_register(unsigned index) { return _xgetbv(index); }

int main(void)
{
    long long xcr0 = _xgetbv(0);
    printf("xcr0 %llx\n", (unsigned long long)xcr0);
    printf("x87 %d sse %d avx %d\n", (int)(xcr0 & 1), (int)(xcr0 >> 1 & 1), (int)(xcr0 >> 2 & 1));
    printf("again %d\n", control_register(0) == xcr0);
    return (xcr0 & 1) ? 0 : 1;
}
