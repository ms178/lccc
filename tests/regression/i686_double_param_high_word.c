/* i686 double-parameter high-word store.
 *
 * The i686 codegen copies stack parameters to local allocas; a double needs
 * TWO 32-bit stores (low word and high word).  In certain slot layouts the
 * high-word store was dropped, leaving the upper half of the double
 * uninitialized: f01(10.0) returned garbage instead of 10 because the
 * converted value read the full 8-byte alloca.  The original report was
 * triggered by a TU that also contained long double declarations
 * (stdlib.h + math.h), so the check functions mix in long double contexts.
 *
 * Every probe uses a double whose LOW 32 bits are zero, so a correct result
 * is impossible unless the high word was stored.  libc-free; exits through
 * the i386 ABI, so it runs on any x86-64 Linux host with an ELF32-capable
 * kernel and needs no 32-bit userspace loader.
 */

/* long double contexts: their presence perturbs slot assignment. */
static long double ld_mix(long double a, long double b)
{
    long double t = a * b + a;
    return t - b;
}

__attribute__((noinline))
static unsigned int f01(double x)
{
    return (unsigned int)x;
}

__attribute__((noinline))
static unsigned int f02(double x, int n)
{
    return (unsigned int)x + (unsigned int)n;
}

__attribute__((noinline))
static unsigned long long f03(double x)
{
    return (unsigned long long)x;
}

static int check_probes(void)
{
    /* 10.0 = 0x40240000_00000000: low word 0, high word carries the value. */
    if (f01(10.0) != 10U)
        return 0;
    /* 1.0 = 0x3FF00000_00000000. */
    if (f01(1.0) != 1U)
        return 0;
    /* 0x43300000_00000000 = 4503599627370496.0 (2^52). */
    if (f03(4503599627370496.0) != 4503599627370496ULL)
        return 0;
    if (f02(10.0, 5) != 15U)
        return 0;
    if (f02(1.0, 1) != 2U)
        return 0;
    return 1;
}

static int check_ld_context(void)
{
    /* Force long double codegen in the same TU, then re-run the probes. */
    volatile long double l = 1.5L;
    long double r = ld_mix(l, 2.0L); /* 1.5*2.0 + 1.5 - 2.0 = 2.5 */
    if ((double)r != 2.5)
        return 0;
    return check_probes();
}

__attribute__((noreturn))
void _start(void)
{
    int status = (check_probes() && check_ld_context()) ? 0 : 1;
    __asm__ volatile ("int $0x80" : : "a"(1), "b"(status) : "memory");
    __builtin_unreachable();
}
