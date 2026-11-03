/* Shared declarations of the AArch64/RV64 floating-point interop fixture
 * (psabi_float_interop_callee.c, psabi_float_interop_caller.c).  Each side is
 * built by LCCC or by Clang and linked against the other, so every placement
 * rule below must match the psABI, not merely LCCC's other half.
 *
 *  - RISC-V hardware-FP convention: a named float or double that finds
 *    fa0-fa7 taken continues in the integer convention -- the next free
 *    a-register, then the stack -- instead of going straight to the stack
 *    (AAPCS64 does go to the stack).  va_start then skips the a-registers
 *    those named arguments took.
 *  - A named `struct { double, double }`, `struct { float, float }` or
 *    `struct { double, long long }` travels in FP (and integer) registers
 *    whatever the callee expression: a function, a function-pointer member,
 *    an array element, a local or a parameter.
 *  - A call through any such pointer to a variadic prototype is variadic: on
 *    RISC-V its unnamed doubles and unnamed float structs use the integer
 *    convention, which is where the callee's va_arg reads them.
 *
 * AAPCS64 passes and returns a homogeneous floating-point aggregate (struct P,
 * struct F) in SIMD registers.  LCCC does not implement HFAs yet -- it uses
 * the general registers on both sides, consistently with itself but not
 * with GCC or Clang -- so the HFA calls run on RV64 only; struct DI is no
 * HFA and runs on both targets. */
#if defined(__aarch64__)
#define HFA_CALLS 0
#else
#define HFA_CALLS 1
#endif
struct P { double x, y; };
struct F { float a, b; };
struct DI { double d; long long i; };

double nine_doubles(double a0, double a1, double a2, double a3, double a4, double a5,
                    double a6, double a7, double a8, long long k);
float ten_floats(int k, float a0, float a1, float a2, float a3, float a4, float a5,
                 float a6, float a7, float a8, float a9);
double nine_then_variadic(double a0, double a1, double a2, double a3, double a4, double a5,
                          double a6, double a7, double a8, ...);
double p_sum(struct P p);
double f_sum(int k, struct F f);
double di_sum(struct DI s);
double v_doubles(int n, ...);
double v_points(int n, ...);
double through_param(double (*cb)(struct P), struct P p);

struct ops {
    double (*p)(struct P);
    double (*f)(int, struct F);
    double (*di)(struct DI);
    double (*v)(int, ...);
    double (*vp)(int, ...);
};
