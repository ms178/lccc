/* Typed-MachInst census admission probe: an over-aligned parameter whose
 * address escapes via a CALL ARGUMENT while the value is also read. Both
 * shapes must flow through the typed census without a rejection — the
 * pre-S11 fallback (reject to the mature path) split the MachInst run and
 * flushed it around the call for every such function.
 *
 * Self-checking under the regression corpus (h(77) == 78, exit 0), and the
 * structural trip-wire for check_overalign_typed_census.sh
 * (compile-only, CCC_ISEL_STATS census at -O1). */
#include <stdint.h>

typedef int A32 __attribute__((aligned(32)));

/* noinline: keeps the &x CALL-ARGUMENT shape alive for the census check. */
__attribute__((noinline)) static int sinkp32(int *p) {
    return (((uintptr_t)p & 31) == 0) ? 1 : 0;
}

__attribute__((noinline)) int h(A32 x) {
    int r = sinkp32(&x);
    return r ? x + 1 : -1;
}

int main(void) {
    return h(77) == 78 ? 0 : 1;
}
