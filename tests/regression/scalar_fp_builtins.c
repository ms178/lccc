// Scalar FP builtins end-to-end (rint/floor/ceil/trunc/copysign/fma/fabs/sqrt)
// on every backend. These lower to RoundScalarF64/FmaScalarF64/CopysignF64
// intrinsics; x86 got SSE lowering while aarch64 and riscv64 silently dropped
// them in their intrinsic catch-alls (every result read uninitialized
// registers -> 0.0). Self-checking so no libm link is needed.
#include <stdio.h>

static int fails = 0;
#define CHECK(cond) do { if (!(cond)) { fails++; } } while (0)

volatile double vx = -2.37519;
volatile double vy = 2.0;
volatile float fx = -2.5f;
volatile float fy = 4.0f;

int main(void) {
    double x = vx, y = vy;
    CHECK(__builtin_rint(x) == -2.0);
    CHECK(__builtin_floor(x) == -3.0);
    CHECK(__builtin_ceil(x) == -2.0);
    CHECK(__builtin_trunc(x) == -2.0);
    CHECK(__builtin_rint(y) == 2.0);
    CHECK(__builtin_floor(y) == 2.0);
    CHECK(__builtin_copysign(1.5, x) == -1.5);
    CHECK(__builtin_copysign(1.5, y) == 1.5);
    CHECK(__builtin_fma(x, y, 0.5) == -4.25038);
    CHECK(__builtin_fabs(x) == 2.37519);
    CHECK(__builtin_sqrt(__builtin_fabs(x)) == __builtin_sqrt(2.37519));
    float f = fx, g = fy;
    CHECK(__builtin_rintf(f) == -2.0f);
    CHECK(__builtin_floorf(f) == -3.0f);
    CHECK(__builtin_copysignf(1.0f, f) == -1.0f);
    CHECK(__builtin_fmaf(f, g, 0.5f) == -9.5f);
    CHECK(__builtin_fabsf(f) == 2.5f);
    if (fails != 0) {
        printf("scalar_fp_builtins: %d check(s) failed\n", fails);
    }
    return fails != 0;
}
