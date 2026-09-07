/*
 * ISA gate corpus for the middle-end vectorizer.
 *
 * `tests/regression/check_vectorize_isa_gate.sh` pins the *instruction choice*
 * for this file; `main()` below pins the *semantics*, so a regression cannot
 * hide by simply stopping vectorization.
 *
 * Root cause (defect C2, kernel-boot session 2026-09): `passes::vectorize`
 * chose its vector width from an environment override alone -- AVX2 was the
 * default and nothing consulted the TU's ISA flags. The Linux kernel is built
 * with `-mno-sse -mno-mmx -mno-sse2 -mno-avx` because it runs with
 * CR4.OSFXSR=0 and never saves FPU state, yet `identify_cpu` in
 * arch/x86/kernel/cpu/common.c acquired `vmovdqu %ymm0` / `vpand %ymm0` from
 * the vectorized `memset(&c->x86_capability, 0, ...)`. That is an immediate
 * #UD on a CPU without AVX2, and clobbers FPU state even where it does not
 * fault.
 *
 * The gate lives in the vectorizer rather than the backend: by the time an
 * intrinsic reaches codegen the scalar loop shape needed for the fallback has
 * already been rewritten away.
 *
 * Integer-only on purpose. Scalar FP under `-mno-sse` is a separate, still
 * open limitation (it needs an x87 path in the backend); including FP here
 * would make the "no SIMD register at all" assertion ambiguous.
 */
#include <stdio.h>

#define N 64

/* Elementwise map: the shape the kernel's masking/`memset` loops lower to. */
__attribute__((noinline)) void and_mask(int *d, const int *m, int n)
{
    for (int i = 0; i < n; i++)
        d[i] &= m[i];
}

__attribute__((noinline)) void add_scaled(int *d, const int *s, int n)
{
    for (int i = 0; i < n; i++)
        d[i] += s[i] * 3;
}

/* Integer dot-product reduction: the other vectorized shape. */
__attribute__((noinline)) long dot_ll(const long *a, const long *b, int n)
{
    long acc = 0;
    for (int i = 0; i < n; i++)
        acc += a[i] * b[i];
    return acc;
}

int main(void)
{
    static int d[N], m[N], s[N];
    static long va[N], vb[N];
    long fail = 0;

    for (int i = 0; i < N; i++) {
        d[i] = i * 7 + 1;
        m[i] = 0x0F0F0F0F ^ (i * 255);
        s[i] = (i % 13) - 6;
        va[i] = (i % 5) - 2;
        vb[i] = (i % 11) - 5;
    }

    /* Expected values, computed straight from the initialisers so the check
     * is independent of whatever the optimizer did to the loops. */
    int want_and[N], want_add[N];
    long want_dot = 0;
    for (int i = 0; i < N; i++) {
        want_and[i] = (i * 7 + 1) & (0x0F0F0F0F ^ (i * 255));
        want_add[i] = want_and[i] + ((i % 13) - 6) * 3;
        want_dot += (long)((i % 5) - 2) * ((i % 11) - 5);
    }

    and_mask(d, m, N);
    add_scaled(d, s, N);
    for (int i = 0; i < N; i++) {
        if (d[i] != want_add[i])
            fail++;
    }
    if (dot_ll(va, vb, N) != want_dot)
        fail++;

    /* A second pass with a different length exercises the scalar remainder /
     * peel path that the vector transforms add. */
    for (int i = 0; i < N; i++)
        d[i] = i * 7 + 1;
    and_mask(d, m, 37);
    for (int i = 0; i < 37; i++)
        if (d[i] != want_and[i])
            fail++;
    for (int i = 37; i < N; i++)
        if (d[i] != i * 7 + 1)
            fail++;

    printf("fail=%ld\n", fail);
    return fail != 0;
}
