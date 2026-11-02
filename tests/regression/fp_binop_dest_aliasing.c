/* FP binop destination-aliasing: the destination register holds an operand.
 *
 * `emit_float_binop_into_reg` leaves the LHS in the XMM home of the result and
 * then applies the RHS. Two ways of materialising the RHS destroyed the LHS:
 *
 *   1. a constant RHS was loaded INTO the destination
 *      (`movsd .LCFP(%rip), %xmm2; vaddsd %xmm2, %xmm2, %xmm2`), so `x + 1.5`
 *      evaluated `1.5 + 1.5` and a `s += a[i]*b[i]` reduction returned the
 *      last product doubled;
 *   2. a homeless value RHS did the same through the GPR path
 *      (`movq %rcx, %xmm2`).
 *
 * A third case predates both: when the destination register is the HOME of the
 * RIGHT operand, loading the LHS into it destroys the RHS. `sub`/`div` must
 * stage the LHS instead of swapping (swapping is only valid for add/mul).
 *
 * Signed zero is checked as well: `0.0 + x` must NOT be folded to `x`
 * (0.0 + -0.0 is +0.0), which is the shortcut a "reduction starts at zero"
 * peephole is tempted to take.
 */
#include <stdint.h>
#include <string.h>

static int fails;

static void chk_d(const char *what, double got, double want) {
    uint64_t g, w;
    memcpy(&g, &got, 8);
    memcpy(&w, &want, 8);
    if (g != w) {
        fails++;
        __builtin_printf("FAIL %s: got %.17g (%016llx) want %.17g (%016llx)\n", what, got,
                         (unsigned long long)g, want, (unsigned long long)w);
    }
}

static void chk_f(const char *what, float got, float want) {
    uint32_t g, w;
    memcpy(&g, &got, 4);
    memcpy(&w, &want, 4);
    if (g != w) {
        fails++;
        __builtin_printf("FAIL %s: got %.9g want %.9g\n", what, (double)got, (double)want);
    }
}

__attribute__((noinline)) static double add_const(double x) { return x + 1.5; }
__attribute__((noinline)) static double sub_const(double x) { return x - 1.5; }
__attribute__((noinline)) static double mul_const(double x) { return x * 3.0; }
__attribute__((noinline)) static double div_const(double x) { return x / 4.0; }
__attribute__((noinline)) static float fadd_const(float x) { return x + 2.5f; }
__attribute__((noinline)) static float fmul_const(float x) { return x * 0.5f; }

/* Reduction whose accumulator lives in an XMM home and whose RHS is a
 * freshly computed product (dot4 shape: `s = 0.0 + p` on the first step). */
__attribute__((noinline)) static double dot4(const double *a, const double *b) {
    double s = 0.0;
    for (int i = 0; i < 4; i++) s += a[i] * b[i];
    return s;
}

/* Non-commutative op whose destination is the home of the RIGHT operand:
 * `acc = x - acc` keeps flipping which side the live register holds. */
__attribute__((noinline)) static double alternating_sub(const double *v, int n) {
    double acc = 1.0;
    for (int i = 0; i < n; i++) acc = v[i] - acc;
    return acc;
}

__attribute__((noinline)) static double alternating_div(const double *v, int n) {
    double acc = 1.0;
    for (int i = 0; i < n; i++) acc = v[i] / acc;
    return acc;
}

/* 0.0 + (-0.0) is +0.0; a load-through fold would return -0.0. */
__attribute__((noinline)) static double zero_plus(double x) { return 0.0 + x; }

int main(void) {
    chk_d("add_const", add_const(1.0), 2.5);
    chk_d("sub_const", sub_const(1.0), -0.5);
    chk_d("mul_const", mul_const(2.0), 6.0);
    chk_d("div_const", div_const(10.0), 2.5);
    chk_f("fadd_const", fadd_const(1.0f), 3.5f);
    chk_f("fmul_const", fmul_const(5.0f), 2.5f);

    const double a[4] = {1.0, 2.0, 3.0, 4.0};
    const double b[4] = {5.0, 6.0, 7.0, 8.0};
    chk_d("dot4", dot4(a, b), 70.0);

    const double v[5] = {10.0, 20.0, 30.0, 40.0, 50.0};
    /* 10-1=9; 20-9=11; 30-11=19; 40-19=21; 50-21=29 */
    chk_d("alternating_sub", alternating_sub(v, 5), 29.0);
    /* 10/1=10; 20/10=2; 30/2=15; 40/15=8/3; 50/(8/3)=18.75 */
    chk_d("alternating_div", alternating_div(v, 5), 50.0 / (40.0 / (30.0 / (20.0 / 10.0))));

    double negzero = -0.0;
    chk_d("zero_plus_negzero", zero_plus(negzero), 0.0);
    chk_d("zero_plus_value", zero_plus(3.25), 3.25);

    return fails;
}
