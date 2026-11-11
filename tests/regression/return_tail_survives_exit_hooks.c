/* Code a compiler inserts "just before the return" has to run before the
   instructions that load the return registers, not between them and `ret`.

   Two inserters, two victims, all four combinations wrong before this file:

   -finstrument-functions   the __cyg_profile_func_exit call was appended
                            after the load of %xmm1 for the imaginary part of
                            a _Complex double (`mk` returned 1.5 + 0i at -O0),
                            and after __builtin_return's reload of
                            %rax/%rdx/%xmm0 (`forward` returned whatever the
                            hook left in %rax).
   -fstack-protector        the canary check ran after __builtin_return's
                            reload and returned the canary (the _ssp variant;
                            the complex return was already handled there).

   The hook below deliberately dirties %rax, %rdx, %xmm0 and %xmm1.  The plain
   run (no flag) checks the same values without any inserted code. */
#include <stdio.h>

#define NOINLINE __attribute__((noinline, noclone))
#define NOHOOK __attribute__((no_instrument_function))

NOHOOK void __cyg_profile_func_enter(void *fn, void *site);
NOHOOK void __cyg_profile_func_exit(void *fn, void *site);

static volatile double fp_dirt = 0.25;
static volatile long int_dirt = 0x5555;
static long hook_calls;

NOHOOK void __cyg_profile_func_enter(void *fn, void *site) {
    (void)fn;
    (void)site;
}

NOHOOK void __cyg_profile_func_exit(void *fn, void *site) {
    (void)fn;
    (void)site;
    double a = fp_dirt + 1e300, b = fp_dirt + 3.5;
    fp_dirt = (a * b - a / b) * 0.0 + 0.25;
    int_dirt = (int_dirt * 31 + 7) | 0x5555;
    hook_calls++;
}

NOINLINE static _Complex double mk(double re, double im) {
    _Complex double z;
    __real__ z = re;
    __imag__ z = im;
    return z;
}

NOINLINE static _Complex float mkf(float re, float im) {
    _Complex float z;
    __real__ z = re;
    __imag__ z = im;
    return z;
}

NOINLINE static long target(long a, long b) {
    volatile char pad[24];
    pad[0] = (char)a;
    return a * 1000 + b + pad[0] - (char)a;
}

/* The arrays give -fstack-protector-strong a reason to guard the frame. */
NOINLINE static long forward(long a, long b) {
    volatile char pad[24];
    pad[1] = (char)(a + b);
    __builtin_return(__builtin_apply((void (*)())target, __builtin_apply_args(), 32));
}

#ifdef __SSE2__
NOINLINE static double target_fp(double x, double y) { return x * 100 + y; }

NOINLINE static double forward_fp(double x, double y) {
    volatile char pad[24];
    pad[2] = (char)(x + y);
    __builtin_return(__builtin_apply((void (*)())target_fp, __builtin_apply_args(), 32));
}
#endif

NOHOOK int main(void) {
    _Complex double z = mk(1.5, 2.5);
    _Complex float w = mkf(3.5f, 4.5f);
    printf("%g %g %g %g\n", __real__ z, __imag__ z, (double)__real__ w, (double)__imag__ w);
    printf("%ld\n", forward(7, 5));
#ifdef __SSE2__
    printf("%g\n", forward_fp(1.5, 0.25));
#endif
    printf("%s\n", hook_calls == 0 || hook_calls >= 4 ? "ok" : "FAIL");
    return 0;
}
