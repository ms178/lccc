/* Shared declarations of the complex / 2×XLEN / RV64-flattening interop
 * fixture (psabi_wide_interop_callee.c, psabi_wide_interop_caller.c).  Each
 * side is built by LCCC or by Clang and linked against the other.
 *
 * A complex argument is ONE aggregate in every psABI, never two scalars:
 *  - AAPCS64: a two-member HFA.  `f(double x0, ..., double x6, _Complex
 *    double z, double t)` finds one SIMD register left, so z goes whole to
 *    the stack (16 bytes; one 8-byte slot for `_Complex float`, 32 bytes
 *    16-aligned for `_Complex long double`) and t follows it there (C.3
 *    closes the SIMD file).  va_arg reads it the same way.
 *  - RV64: a named one is "a struct containing two floating-point reals":
 *    fa-pair, else the integer convention (a0/a1 for `_Complex double`).
 *  - x86-64: `_Complex double` is a `[SSE, SSE]` record -- with one %xmm
 *    left, both halves go to the stack.
 *
 * RV64 2×XLEN scalars (`__int128`, `long double`) and aggregates that find
 * only a7 free are split: low half in a7, high half at 0(sp).  A variadic
 * one starts at an even register instead and so skips a7 for the stack.
 *
 * RV64 flattening (GCC 10+ and Clang agree on all of these): a pointer
 * member disqualifies the record (`struct DP` uses a0/a1); a bit-field is an
 * integer member (`struct FB` is fa0 + a0); a lone complex member is two FP
 * members (`struct CZ` is fa0/fa1); a complex after another member
 * disqualifies (`struct FZ` uses a0/a1).
 *
 * long double values are only moved and compared bit-for-bit (arithmetic on
 * them needs libgcc), and __int128 only through shifts, xor and add. */
#ifndef PSABI_WIDE_INTEROP_H
#define PSABI_WIDE_INTEROP_H

struct DP { double d; void *p; };
struct FB { float f; int b : 3; };
struct CZ { _Complex float z; };
struct CD { _Complex double z; };
struct FZ { float a; _Complex float z; };
struct S2 { long a, b; };

double cd_after7(double x0, double x1, double x2, double x3, double x4, double x5, double x6,
                 _Complex double z, double tail);
double cf_after7(double x0, double x1, double x2, double x3, double x4, double x5, double x6,
                 _Complex float z, double tail);
double cd_after8(double x0, double x1, double x2, double x3, double x4, double x5, double x6,
                 double x7, _Complex double z, double tail);
unsigned long long cld_after7(double x0, double x1, double x2, double x3, double x4, double x5,
                              double x6, _Complex long double z, double tail);
unsigned long long cld_bits(_Complex long double z, int k);
double cplx_mix(float f, _Complex float a, double d, _Complex double b, long k);
double va_cplx(int n, ...);
_Complex double ret_cd(double re, double im);
_Complex float ret_cf(float re, float im);

unsigned long long ld_a7(long a0, long a1, long a2, long a3, long a4, long a5, long a6,
                         long double x, long after);
__int128 i128_a7(long a0, long a1, long a2, long a3, long a4, long a5, long a6, __int128 x,
                 long after);
long s2_a7(long a0, long a1, long a2, long a3, long a4, long a5, long a6, struct S2 s,
           long after);
unsigned long long va_ld(int n, ...);
__int128 va_i128(int n, ...);

double sum_dp(struct DP s);
double sum_fb(struct FB s);
double sum_cz(struct CZ s);
double sum_cdm(struct CD s);
double sum_fz(struct FZ s);
struct DP ret_dp(double d, void *p);
struct FB ret_fb(float f, int b);
struct CZ ret_cz(float re, float im);
struct CD ret_cdm(double re, double im);
struct FZ ret_fz(float a, float re, float im);

#endif
