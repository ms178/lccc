/* A dynamic-TLS access combined with a wide-type operation.
 *
 * On RISC-V, __int128 division lowers to a hidden `call __divti3`, which
 * clobbers a0-a7 and t0-t6. The dynamic-TLS resolver is also a hidden call,
 * and the code generator used to park its result in a0 whenever the only
 * IR-VISIBLE call was that resolver. `__divti3` is not IR-visible, so the
 * address was destroyed between the resolver and its last use and `combine`
 * returned the quotient twice instead of address plus quotient -- a silent
 * miscompile that no relocation count could reveal.
 *
 * The gate asserts this file two ways: the answer at runtime, and the register
 * discipline statically (the address must be parked in a callee-saved register
 * and the function must get a real s0 frame), so the contract holds even where
 * the runtime half has to skip. */
extern __thread unsigned long external_tls;

__int128 g_a = 0x123456789abcdef0LL;
__int128 g_b = 3;

__attribute__((noinline)) unsigned long combine(void) {
    unsigned long *p = &external_tls; /* dynamic TLS: hidden resolver call */
    __int128 q = g_a / g_b;      /* hidden call __divti3: clobbers a0 */
    return (unsigned long)p + (unsigned long)q;
}

int main(void) {
    unsigned long *p = &external_tls;
    __int128 q = g_a / g_b;
    if (combine() != (unsigned long)p + (unsigned long)q)
        return 1; /* the resolver's result was clobbered by the wide call */
    return 0;
}
