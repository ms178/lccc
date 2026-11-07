/* Cross-ISA fixture 2: a RUNTIME (volatile-derived) 16-byte image.
 *
 * No compiler may fold this movemask: the bytes come from a volatile read,
 * so the object-image proof must refuse.  This exercises the AArch64 NEON
 * reduction and the RISC-V software reduction for real, under QEMU, and
 * compares BOTH pass states against an independent Python oracle.  A
 * miscompile in either backend, or an over-eager fold, changes the exit
 * status and is caught. */
typedef long long v2i64 __attribute__((vector_size(16)));
extern int __lccc_simd128_i_pmovmskb128(long, v2i64);

volatile unsigned char seed = 0x01;
union U { v2i64 v; unsigned char b[16]; };

int lccc_main(void) {
    unsigned char s = seed;
    union U u;
    u.b[0]  = (unsigned char)(s * 1u * 37u + 0x40u);
    u.b[1]  = (unsigned char)(s * 2u * 37u + 0x40u);
    u.b[2]  = (unsigned char)(s * 3u * 37u + 0x40u);
    u.b[3]  = (unsigned char)(s * 4u * 37u + 0x40u);
    u.b[4]  = (unsigned char)(s * 5u * 37u + 0x40u);
    u.b[5]  = (unsigned char)(s * 6u * 37u + 0x40u);
    u.b[6]  = (unsigned char)(s * 7u * 37u + 0x40u);
    u.b[7]  = (unsigned char)(s * 8u * 37u + 0x40u);
    u.b[8]  = (unsigned char)(s * 9u * 37u + 0x40u);
    u.b[9]  = (unsigned char)(s * 10u * 37u + 0x40u);
    u.b[10] = (unsigned char)(s * 11u * 37u + 0x40u);
    u.b[11] = (unsigned char)(s * 12u * 37u + 0x40u);
    u.b[12] = (unsigned char)(s * 13u * 37u + 0x40u);
    u.b[13] = (unsigned char)(s * 14u * 37u + 0x40u);
    u.b[14] = (unsigned char)(s * 15u * 37u + 0x40u);
    u.b[15] = (unsigned char)(s * 16u * 37u + 0x40u);
    int m = __lccc_simd128_i_pmovmskb128(0, u.v);
    return ((m >> 8) ^ m) & 0xFF;
}
