/* Cross-ISA fixture 1: a PROVABLY CONSTANT 16-byte sign-bit image.
 *
 * `maskfacts` must fold `__lccc_simd128_i_pmovmskb128` to one immediate on
 * x86-64, i686, AArch64 and RISC-V64.  With the pass disabled the backend
 * must emit its real reduction (x86 `pmovmskb`, AArch64 NEON sequence,
 * RISC-V software reduction) and produce the SAME answer.
 *
 * Freestanding: no libc, no headers, no `main`.  The harness supplies an
 * architecture-specific `_start` that calls `lccc_main` and exits with its
 * return value. */
typedef long long v2i64 __attribute__((vector_size(16)));
extern int __lccc_simd128_i_pmovmskb128(long, v2i64);

union U { v2i64 v; unsigned char b[16]; };

int lccc_main(void) {
    union U u;
    u.b[0] = 0x80; u.b[1] = 0x00; u.b[2] = 0x80; u.b[3] = 0x00;
    u.b[4] = 0x80; u.b[5] = 0x00; u.b[6] = 0x80; u.b[7] = 0x00;
    u.b[8] = 0x00; u.b[9] = 0x80; u.b[10] = 0x00; u.b[11] = 0x80;
    u.b[12] = 0x00; u.b[13] = 0x80; u.b[14] = 0x00; u.b[15] = 0x80;
    int m = __lccc_simd128_i_pmovmskb128(0, u.v); /* oracle: 0xAA55 */
    return ((m >> 8) ^ m) & 0xFF;               /* oracle: 0xFF */
}
