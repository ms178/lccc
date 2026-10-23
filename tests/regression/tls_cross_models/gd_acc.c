/* Values cached in a scratch register across a dynamic-TLS address.
 *
 * RISC-V GD materialises `&ext_tls` as `call __tls_get_addr`, which may
 * clobber every caller-saved register -- t0 included, the backend's
 * accumulator. `w2` computes `v` into t0, takes the TLS address into an
 * allocated register, then stores `v`: the codegen register cache still
 * claimed t0 held `v`, so the store wrote whatever the resolver left in t0.
 * glibc's bound fast path happens to preserve t0; the lazy-binding PLT0
 * stub does not (it loads the resolver arguments through t0), so the gate
 * runs the host with LD_BIND_NOT=1 to route every call through it.
 *
 * On AArch64 the same shapes exercise the TLSDESC `blr`: no caller-saved
 * home may span it (the text peephole models the `blr` as a full call). */
extern __thread long ext_tls;

__attribute__((noinline)) long w1(long *q, long n) {
    long v = q[0] * n;
    long *p = &ext_tls;
    return v + *p + v * 3;
}

__attribute__((noinline)) long w2(long a, long b) {
    long v = a ^ b;
    long *p = &ext_tls;
    *p = v;
    return v + 1;
}

__attribute__((noinline)) long w3(long a, long b, long c) {
    long v = a + b;
    long *p = &ext_tls;
    long r = *p;
    return v * c + r + v;
}
