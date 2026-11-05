/* Shared declarations of the dirty-upper-bits interop fixture
 * (abi_dirty_upper_interop_callee.c, abi_dirty_upper_interop_caller.c).
 * Each side is built by LCCC or by Clang and linked against the other.
 *
 * AAPCS64 passes an integral argument narrower than 64 bits in the least
 * significant bits of x[n] (C.16) and returns one in the least significant
 * bits of x0; every other bit of the register is unspecified.  Real code
 * produces such bits: `int f(int a, int b, long k) { return g(a, b, k); }`
 * with `int g(int, int, int)` is a bare `b g` from GCC and Clang alike, so
 * g's third argument arrives carrying k's upper half.  The caller here
 * passes such bits deliberately, through asm-label aliases whose prototypes
 * widen the narrow parameters (and the narrow results) to 64 bits; the
 * callee is compiled against the true narrow prototypes and must read only
 * the bits the ABI defines -- in every consumer: a select, a branch, a
 * widening, a switch index, an address, a compare, a division and a value
 * that lives across a call.  The reverse direction holds too: an `int`
 * result may come back with a dirty upper half, and the caller must ignore
 * it.
 *
 * RV64's psABI instead requires integers narrower than XLEN to arrive (and
 * return) widened to 32 bits by the sign of their type and then
 * sign-extended to XLEN, so on RISC-V the caller passes exactly that and
 * the same checks run on clean bits. */
#ifndef ABI_DIRTY_UPPER_INTEROP_H
#define ABI_DIRTY_UPPER_INTEROP_H

#if defined(__aarch64__)
/* The upper bits a dirty AAPCS64 caller leaves behind. */
#define DU_GARBAGE32 0x5a5a5a5a00000000UL
#define DU_GARBAGE16 0xa5a5a5a5a5a50000UL
#define DU_GARBAGE8 0xc3c3c3c3c3c3c300UL
#define DU_S32(v) ((long)(DU_GARBAGE32 | (unsigned int)(v)))
#define DU_U32(v) ((long)(DU_GARBAGE32 | (unsigned int)(v)))
#define DU_S16(v) ((long)(DU_GARBAGE16 | (unsigned short)(v)))
#define DU_U16(v) ((long)(DU_GARBAGE16 | (unsigned short)(v)))
#define DU_S8(v) ((long)(DU_GARBAGE8 | (unsigned char)(v)))
#define DU_U8(v) ((long)(DU_GARBAGE8 | (unsigned char)(v)))
#else
/* RV64 psABI: widen to 32 bits by the type's sign, then sign-extend. */
#define DU_S32(v) ((long)(int)(v))
#define DU_U32(v) ((long)(int)(unsigned int)(v))
#define DU_S16(v) ((long)(short)(v))
#define DU_U16(v) ((long)(unsigned short)(v))
#define DU_S8(v) ((long)(signed char)(v))
#define DU_U8(v) ((long)(unsigned char)(v))
#endif

#endif
