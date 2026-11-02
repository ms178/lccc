/* A separately compiled, typed SysV 256-bit argument witness. No intrinsic
 * lowering or LCCC calling-convention assumption is used to build the oracle. */
#include <stdio.h>
typedef double vector4d __attribute__((vector_size(32)));
extern void caller_upper_call(const double *, double *);
__attribute__((noinline, noipa)) void wide_witness(vector4d value, double *out) {
    __builtin_memcpy(out, &value, sizeof(value));
}
int main(void) {
    const double input[5] = { 1.0, 2.0, 3.0, 4.0, 8.0 };
    const double expected[4] = { 9.0, 2.0, 3.0, 4.0 };
    double out[4] = { -1.0, -1.0, -1.0, -1.0 };
    caller_upper_call(input, out);
    printf("%.0f %.0f %.0f %.0f\n", out[0], out[1], out[2], out[3]);
    return __builtin_memcmp(out, expected, sizeof(out)) != 0;
}
