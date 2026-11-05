/* Shared declarations of the by-reference aggregate interop fixture
 * (psabi_byref_interop_callee.c, psabi_byref_interop_caller.c).  Each side
 * is built by LCCC or by Clang and linked against the other.
 *
 * AAPCS64 (B.4) and the RISC-V psABI pass a composite larger than 16 bytes
 * -- and RV64 a `_Complex long double` -- as a pointer to a COPY made by the
 * caller.  The callee owns that memory and may write to it in place; GCC and
 * Clang do so without copying it again.  A caller that hands over the
 * address of its own object therefore sees the callee's writes, and one that
 * passes the same object twice, or an object the callee also reaches through
 * a global, changes what the callee reads.  x86-64 copies these arguments to
 * the stack (MEMORY class); the same checks hold there.
 *
 * A record returned through a hidden buffer must not alias an argument
 * either: `x = big_rev(x)` with a callee that fills its result while it is
 * still reading the argument.
 *
 * long double values are only moved and compared bit for bit (arithmetic on
 * them needs libgcc). */
#ifndef PSABI_BYREF_INTEROP_H
#define PSABI_BYREF_INTEROP_H

struct Big { long a[5]; };
struct Mix { double d; long l[3]; };

extern struct Big gbig;

long big_mod(struct Big b);
long big_two(struct Big p, struct Big q);
long big_glob(struct Big b);
struct Big big_rev(struct Big b);
long big_after8(long r0, long r1, long r2, long r3, long r4, long r5, long r6, long r7,
                struct Big b, long tail);
long mix_mod(double f, struct Mix m, struct Mix n);
long cld_mod(_Complex long double z, _Complex long double w);
long big_va(int n, ...);

#endif
