// fmsub/fnmsub fusion (levkropp e3b21b8f, audited port): under
// -ffp-contract=fast, `c - a*b` -> fmsub and `a*b - c` -> fnmsub on
// AArch64 (vfnmadd231/vfmsub231 on x86). Values are exact in binary64,
// so contracted and uncontracted results are bit-identical here — the
// test passes at any contract setting and validates operand order.
int printf(const char *, ...);
double s1(double a, double b, double c) { return c - a * b; }
double s2(double a, double b, double c) { return a * b - c; }
int main(void) {
    volatile double a = 3, b = 4, c = 20;
    printf("%.1f %.1f\n", s1(a, b, c), s2(a, b, c));
    return 0;
}
