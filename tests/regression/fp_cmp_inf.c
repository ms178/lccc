/* FP comparisons with a register-allocated second operand.
 * emit_fp_operand_to_xmm ignored register assignments and read %rcx (garbage)
 * when the acc cache claimed the value — so `inf != inf` evaluated TRUE at -O0
 * (frexp_volatile_global: glibc's frexp test, rc=2). The fix honors XMM/GPR
 * register assignments first and sources the accumulator from %rax only.
 *
 * This is the minimal glibc-style shape: compute x = 1.0/0.0 (inf), copy it,
 * then compare the copies — exactly the frexp(x) != x check. */
#include <stdio.h>

int main(void) {
    double x = 1.0 / 0.0;   /* +inf */
    double fx = x;
    int ne = (fx != x);
    int eq = (fx == x);
    int lt = (fx < x);      /* inf < inf must be false */
    int gt = (fx > x);      /* inf > inf must be false */
    if (ne != 0 || eq != 1 || lt != 0 || gt != 0) {
        printf("FAIL ne=%d eq=%d lt=%d gt=%d\n", ne, eq, lt, gt);
        return 1;
    }
    /* and the -0.0 / memcmp shape from the frexp test */
    double minus_zero = -0.0;
    x = minus_zero;
    double y = x;
    unsigned char *a = (unsigned char *)&y, *b = (unsigned char *)&x;
    for (int i = 0; i < 8; i++) if (a[i] != b[i]) return 2;
    printf("PASS fp_cmp_inf\n");
    return 0;
}
