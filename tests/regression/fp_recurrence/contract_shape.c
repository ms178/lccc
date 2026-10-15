/* Positive contraction/ISA witness: deliberately no FENV_ACCESS pragma.
 * The FENV-enabled cases.c supplies the separate mandatory refusal arm. */
__attribute__((noinline)) double twice_plus(double x, double y) {
    return x * 2.0 + y;
}
__attribute__((noinline)) double scaled_fma(double x, double y, double z) {
    return 2.0 * x * y + z;
}
