/* Typed scalar-float MachInst layer (FMov / FAlu).
 *
 * Exercises the shapes the typed lowering owns: xmm-homed F32/F64 values
 * copied register-to-register, stored/loaded through alloca slots and GPR
 * pointers, and combined with the VEX three-operand vadd/vsub/vmul/vdiv
 * forms -- including the operand-order-sensitive cases (sub/div keep the
 * lhs in src1) and the commutative swap when only the rhs is register
 * homed. Results are printed bit-exactly so any mnemonic-size mixup
 * (movss vs movsd) or operand swap shows up as a diff against the oracle
 * build.
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

static double gd[4];
static float gf[4];
volatile double vd;
volatile float vf;

static uint64_t dbits(double d) {
    uint64_t u;
    memcpy(&u, &d, 8);
    return u;
}
static uint32_t fbits(float f) {
    uint32_t u;
    memcpy(&u, &f, 4);
    return u;
}

double f64_chain(double a, double b) {
    double t = a * b; /* vmulsd */
    double u = t + a; /* vaddsd */
    double w = u - b; /* vsubsd, lhs must stay src1 */
    double q = w / a; /* vdivsd, lhs must stay src1 */
    double c = q;     /* xmm->xmm copy */
    gd[0] = c;        /* store through a global */
    return c + gd[0];
}

float f32_chain(float a, float b) {
    float t = a * b;
    float u = t + a;
    float w = u - b;
    float q = w / a;
    float c = q;
    gf[0] = c;
    return c + gf[0];
}

/* Store + load round-trip through volatile memory (no fold-through). */
double slot_roundtrip(double x) {
    double local = x * 2.0;
    vd = local;
    double back = vd;
    return back - local; /* exactly 0.0 */
}

float slot_roundtrip_f(float x) {
    float local = x * 3.0f;
    vf = local;
    float back = vf;
    return back - local;
}

/* Phi-driven copies: the copy set changes per iteration. */
double phi_copies(double a, double b, int n) {
    double acc = a;
    for (int i = 0; i < n; i++) {
        double t = acc;
        acc = (i & 1) ? t + b : t - b;
    }
    return acc;
}

/* Only the rhs stays register-homed across the branch: commutative ops
 * must swap (rhs into src1) while sub/div keep their order. */
double mixed_homing(double a, double b, int n) {
    double acc = 0.0;
    for (int i = 0; i < n; i++) {
        double lhs = (i & 1) ? a : b; /* slot pressure on lhs */
        acc = acc + lhs * b;
        acc = acc - lhs;
    }
    return acc;
}

int main(void) {
    gd[0] = gd[1] = gd[2] = gd[3] = 0.0;
    gf[0] = gf[1] = gf[2] = gf[3] = 0.0f;

    printf("%.17g\n", f64_chain(3.0, 4.0));
    printf("%.9g\n", f32_chain(2.5f, 1.5f));
    printf("%.17g\n", slot_roundtrip(1.25));
    printf("%.9g\n", slot_roundtrip_f(0.75f));
    printf("%.17g\n", phi_copies(10.0, 0.5, 6));
    printf("%.17g\n", mixed_homing(1.5, 2.25, 5));

    /* Signed zero: (+0.0) + (-0.0) == +0.0 -- the sign bit is observable
     * and a swapped/mis-sized add can change it. */
    double pz = vd - vd;         /* +0.0 through memory */
    double nz = -pz - pz - pz;   /* -0.0: -(+0.0) is -0.0; -0.0-0.0-0.0 stays -0.0 */
    double sum = pz + nz;
    printf("%016llx\n", (unsigned long long)dbits(sum));

    /* F32 and F64 homes must not cross-wire: the same bit pattern means
     * different values at the two widths. */
    float sf = 1.0f;
    double df = 1.0;
    printf("%08llx %016llx\n", (unsigned long long)fbits(sf + sf),
           (unsigned long long)dbits(df + df));
    return 0;
}
