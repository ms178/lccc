/* Freestanding differential checksum for narrow population counts and parity.
 *
 * The counts of a `u8`/`u16` operand are now emitted at the operand's own
 * width: the frontend's zero extension is a `Copy`, and each backend counts a
 * byte or a halfword with its own shorter sequence.  Every one of those
 * sequences reads a *register*, so the question this file answers is whether
 * the bits a narrow type does not own stay out of the answer -- for an
 * unsigned value the emitters mask, and for a signed one the argument is
 * sign-extended before the count sees it, which is the case that a mask-only
 * lowering gets wrong.
 *
 * The checksum was measured with GCC 16.2 and Clang 23.1.5 on this source (all
 * three optimization levels of the hosted compilers agree); `check.c` pins it.
 * `-ffreestanding` and no library calls: the object is linked with LCCC's own
 * start stub and executed under QEMU for the non-host backends.
 */
typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned int u32;
typedef unsigned long long u64;
typedef signed char i8;
typedef signed short i16;

static u64 mix(u64 h, u64 v) {
    return h ^ (v + 0x9e3779b97f4a7c15ull + (h << 6) + (h >> 2));
}

/* One narrow value through all four count questions, at both signednesses. */
static u64 observe(u64 h, u32 raw, u32 bits) {
    h = mix(h, (u64)__builtin_popcount((u8)raw));
    h = mix(h, (u64)__builtin_parity((u8)raw));
    h = mix(h, (u64)__builtin_popcount((i8)raw));
    h = mix(h, (u64)__builtin_parity((i8)raw));
    if (bits == 16) {
        h = mix(h, (u64)__builtin_popcount((u16)raw));
        h = mix(h, (u64)__builtin_parity((u16)raw));
        h = mix(h, (u64)__builtin_popcount((i16)raw));
        h = mix(h, (u64)__builtin_parity((i16)raw));
    }
    return h;
}

u64 count_narrow_checksum(void) {
    u64 h = 0xcbf29ce484222325ull;
    u32 i;
    /* Every byte value, signed and unsigned. */
    for (i = 0; i < 256; i++) {
        h = observe(h, i, 8);
    }
    /* A strided sweep of the halfwords: every residue of the stride, both
     * halves of the type, and the sign-extension boundary. */
    for (i = 0; i < 65536u; i += 97u) {
        h = observe(h, i, 16);
    }
    /* Single bits at every position of both widths, and their complements. */
    for (i = 0; i < 16u; i++) {
        h = observe(h, 1u << i, 16);
        h = observe(h, ~(1u << i) & 0xffffu, 16);
    }
    return h;
}
