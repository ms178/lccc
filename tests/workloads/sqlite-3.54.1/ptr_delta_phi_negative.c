/*
 * A second same-base, constant-offset pointer-phi shape: unlike the varint
 * decoder it has a negative arm, so the address can lower to pointer-width
 * Sub(base, 7) rather than Add/GEP. The branch-local volatile stores keep
 * the merge as an SSA phi instead of an if-converted Select. No libc, headers,
 * or cross sysroot are needed for the four-ISA freestanding runtime oracle.
 */
typedef unsigned char u8;

__attribute__((noinline))
long guarded_byte_delta(const u8 *p, int choice, volatile u8 *out) {
    const u8 *start = p;
    if (choice == 1) {
        p -= 7;
        *out = *p;
    } else if (choice == 2) {
        p += 11;
        *out = *p;
    } else {
        *out = *p;
    }
    return p - start;
}
