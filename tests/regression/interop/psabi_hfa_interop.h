/* Shared declarations of the floating-point aggregate interop fixture
 * (psabi_hfa_interop_callee.c, psabi_hfa_interop_caller.c).  Each side is
 * built by LCCC or by Clang and linked against the other, so every placement
 * rule must match the psABI, not merely LCCC's other half.
 *
 * AAPCS64 (B.2, C.2-C.4, returns, Linux variadics):
 *  - A homogeneous floating-point or short-vector aggregate (HFA/HVA: one to
 *    four members of one base type -- half, float, double, quad, 64- or
 *    128-bit vector -- with no padding; nested records, arrays, unions and
 *    zero-width bit-fields included) travels one member per SIMD register
 *    when that many are free, at ANY size: struct D4 (32 bytes) and struct
 *    LD2 (two quads) are never passed by reference.
 *  - Otherwise it is copied to the stack (size rounded up to 8, 16-aligned
 *    for a 16-byte base) and NO later argument takes a SIMD register: the
 *    float after it goes to the stack too.  The stack alignment is the
 *    members' (capped at 16): a record-level `aligned(32)` does not raise
 *    it, so struct DA32 after nine doubles sits at 8 mod 16, named or not
 *    (gcc.c-torture pr92904).
 *  - It is returned in v0..v3 -- never through x8.
 *  - Unnamed (variadic) HFAs are passed exactly like named ones; va_arg
 *    reads each member from its own 16-byte slot of the register save area,
 *    or the memory image from the stack once the slots run out.
 *  - A bare 8- or 16-byte GNU vector is a one-member HVA.
 *  - Mixed float/double records, five floats, and records with padding are
 *    no HFA: general registers, or by reference past 16 bytes.
 *
 * RISC-V LP64D: a named struct of one or two FP members (or one FP and one
 * integer member) is passed in fa-registers (and an a-register), and is
 * RETURNED the same way: struct D2 in fa0/fa1, struct FI in fa0+a0,
 * struct ID in a0+fa0.  A `_Complex double` before such a struct must not
 * shift its classification onto the complex's imaginary half.
 *
 * `struct ZB { float a; int : 0; float b; }` is exercised on AArch64 only:
 * the RISC-V psABI ignores zero-length bit-fields while flattening (two
 * floats: fa0/fa1, as GCC 10+ and LCCC pass it), but Clang 23 keeps its
 * pre-GCC-10 quirk and uses the integer convention, so no RV64 Clang link
 * can agree with the psABI on it.
 *
 * Every value is a small dyadic rational, so each weighted sum below is
 * exact in binary floating point.  long double and _Float16 members are only
 * moved and compared bit-for-bit: arithmetic on them would need libgcc,
 * which this freestanding fixture does not link. */
#ifndef PSABI_HFA_INTEROP_H
#define PSABI_HFA_INTEROP_H

#if defined(__aarch64__)
#define ZB_INTEROP 1
#else
#define ZB_INTEROP 0
#endif

struct F2 { float a, b; };
struct F3 { float a, b, c; };
struct F4 { float a, b, c, d; };
struct D2 { double a, b; };
struct D3 { double a, b, c; };
struct D4 { double a, b, c, d; };
struct __attribute__((aligned(32))) DA32 { double a, b, c, d; };
struct LD1 { long double a; };
struct LD2 { long double a, b; };
struct NF { struct F2 in; float c; };          /* nested: HFA float x 3 */
struct AD { double v[3]; };                    /* array: HFA double x 3 */
union UD { double v[2]; struct D2 s; };        /* union: HFA double x 2 */
struct ZB { float a; int : 0; float b; };      /* zero-width bit-field */
struct H3 { _Float16 a, b, c; };               /* HFA half x 3 */
typedef float v4f __attribute__((vector_size(16)));
typedef float v2f __attribute__((vector_size(8)));
typedef int v4i __attribute__((vector_size(16)));
struct V2 { v4f x, y; };                       /* HVA 128-bit x 2 */
struct VM { v2f x; v2f y; };                   /* HVA 64-bit x 2 */
/* No HFA anywhere. */
struct MX { float a; double b; };
struct F5 { float a, b, c, d, e; };
/* RISC-V flattened float + integer records. */
struct FI { float f; int i; };
struct ID { int i; double d; };
struct CF { char c; float f; };
struct DS { double d; short s; };
struct D1 { double d; };

/* Arguments. */
double sum_f2(struct F2 s);
double sum_f3(struct F3 s);
double sum_f4(struct F4 s);
double sum_d2(struct D2 s);
double sum_d3(struct D3 s);
double sum_d4(struct D4 s);
double sum_nf(struct NF s);
double sum_ad(struct AD s);
double sum_ud(union UD s);
double sum_zb(struct ZB s);
unsigned sum_h3(struct H3 s);
unsigned long long bits_ld1(struct LD1 s);
unsigned long long bits_ld2(int k, struct LD2 s);
double sum_v4f(v4f v);
double sum_v2f(v2f v);
long long sum_v4i(v4i v);
double sum_v2(struct V2 s);
double sum_vm(struct VM s);
double sum_mx(struct MX s);
double sum_f5(struct F5 s);
double sum_fi(struct FI s);
double sum_id(struct ID s);
double sum_cf(struct CF s);
double sum_ds(struct DS s);
double mix_regs(int i, struct D3 a, float f, struct F4 b, long long l, double d);
double nsrn_overflow(double x0, double x1, double x2, double x3, double x4, double x5,
                     struct D3 s, double after, int k);
double nsrn_overflow_f2(double x0, double x1, double x2, double x3, double x4, double x5,
                        double x6, struct F2 s, float g);
double quads_in_regs(struct D4 a, struct D4 b);
double quads_spill(struct D4 a, struct D4 b, struct D4 c, double tail);
unsigned long long ld_spill(double x0, double x1, double x2, double x3, double x4, double x5,
                            double x6, struct LD2 s, struct LD1 t);
double cplx_then_struct(_Complex double z, struct D2 p, struct FI q);
double cplx_after7(double x0, double x1, double x2, double x3, double x4, double x5, double x6,
                   _Complex double z, double tail);

/* Returns. */
struct F2 ret_f2(float a, float b);
struct F3 ret_f3(float a, float b, float c);
struct F4 ret_f4(float a);
struct D2 ret_d2(double a, double b);
struct D3 ret_d3(double a);
struct D4 ret_d4(double a);
struct NF ret_nf(float a);
struct AD ret_ad(double a);
union UD ret_ud(double a);
struct H3 ret_h3(unsigned short a);
struct LD2 ret_ld2(unsigned long long a, unsigned long long b);
v4f ret_v4f(float a);
v2f ret_v2f(float a);
struct V2 ret_v2(float a);
struct VM ret_vm(float a);
struct MX ret_mx(double a);
struct F5 ret_f5(float a);
struct FI ret_fi(float f, int i);
struct ID ret_id(int i, double d);
struct CF ret_cf(char c, float f);
struct DS ret_ds(double d, short s);
struct D1 ret_d1(double d);
struct D4 scale_d4(struct D4 s, double k);
struct F3 rotate_f3(struct F3 s);
double call_maker(struct D3 (*mk)(double), double x);

/* Variadic. */
double va_hfa(int n, ...);
double va_spill(int n, ...);
double va_named_hfa(struct D3 s, ...);
double va_named_overflow(double x0, double x1, double x2, double x3, double x4, double x5,
                         struct D3 s, ...);
double va_da32(int n, ...);
double da32_after9(double x0, double x1, double x2, double x3, double x4, double x5, double x6,
                   double x7, double x8, struct DA32 s, double tail);

#endif
