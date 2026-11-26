/*
 * A backward step written as `p += -7`. Unlike `p -= 7`, this can reach
 * pointer-delta SSA as GEP(base, -7) or Add(base, -7), both of which used to
 * bypass the measured-slower Sub(base, 7) negative-control gate. The volatile
 * stores retain branch-local memory effects and make the pointer join real.
 * Keep all input pointers at least seven bytes past their object start.
 * No libc, headers or target sysroot are needed for the freestanding oracle.
 */
typedef unsigned char u8;

__attribute__((noinline))
long guarded_negative_add_delta(const u8 *p, int choice, volatile u8 *out) {
    const u8 *start = p;
    if (choice == 1) {
        p += -7;
        *out = *p;
    } else if (choice == 2) {
        p += 11;
        *out = *p;
    } else {
        *out = *p;
    }
    return p - start;
}
