/* Shared declarations of the AArch64/RV64 register-pair interop fixture
 * (psabi_pair_interop_callee.c, psabi_pair_interop_caller.c).  Each side is
 * built by LCCC or by Clang and linked against the other, so every
 * placement rule below must match the psABI, not merely LCCC's other half.
 *
 *  - AAPCS64 C.8 rounds the next general register up to an even number for
 *    an argument whose *natural* alignment is 16.  A composite's natural
 *    alignment is that of its most-aligned member; a struct-level
 *    `aligned(16)` raises the object's alignment but not its natural one
 *    (GCC's aarch64_function_arg_alignment, Clang alike), so `struct T`
 *    takes x1/x2 after one int while `struct N`, whose member is a `T`,
 *    takes x2/x3.  va_arg rounds __gr_offs by the same rule.
 *  - The RISC-V psABI aligns a 2*XLEN-aligned argument to an even register
 *    pair only when it is variadic: a named `__int128` or `struct T` after
 *    one int takes a1/a2.  A named floating-point argument of a variadic
 *    function travels in an FP register; only the variadic ones use the
 *    integer convention. */
struct __attribute__((aligned(16))) T { long long a, b; };
struct N { struct T t; };

long long named_t(int a, struct T t);
long long named_t_late(int a, int b, int c, struct T t);
long long named_n(int a, struct N n);
long long named_i128(int a, __int128 x);
long long named_i128_late(int a, int b, int c, int d, int e, __int128 x);
long long variadic_t(int skip, ...);
long long variadic_n(int skip, ...);
long long variadic_i128(int skip, ...);
double named_double_then_variadic(double d, ...);
long long named_int_double_then_variadic(int a, double d, ...);

#define T_A 0x0123456789abcdefll
#define T_B (-0x1122334455667788ll)
#define I128_HI 0x0f1e2d3c4b5a6978ll
#define I128_LO 0x8796a5b4c3d2e1f0ull
#define MIX(a, b, k) ((long long)((unsigned long long)(a) * 3u + (unsigned long long)(b) + (unsigned long long)(k)))
