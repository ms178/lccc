/* Accepted calling conventions belong in the positive IR sweep. Foreign
 * preservation conventions are rejected before IR; the ABI matrix and
 * check_attribute_diagnostics.sh separately pin those negative controls. */
#if defined(__x86_64__)
#define NATIVE __attribute__((sysv_abi))
#else
#define NATIVE __attribute__((cdecl))
#endif
#define NOINLINE __attribute__((noinline))
typedef long (*native_fn)(long, long);
NATIVE NOINLINE long native_sum(long a, long b) { return a + b; }
NATIVE NOINLINE long native_forward(long a, long b) {
    return native_sum(a + 1, b + 2);
}
NATIVE NOINLINE long native_indirect(native_fn fn, long a, long b) {
    return fn(a, b);
}
int main(void) {
    if (native_sum(7, 9) != 16) return 1;
    if (native_forward(7, 9) != 19) return 2;
    return native_indirect(native_sum, 4, 5) != 9;
}
