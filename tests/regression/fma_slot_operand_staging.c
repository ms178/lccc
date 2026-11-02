/* In-place FMA231 with slot-resident multiplicands (x86-64, -mavx -mfma).
 *
 * Under XMM pressure the allocator leaves short temporaries in stack slots.
 * `emit_scalar_fma231_with` then forms `acc + a*b` in the accumulator's home
 * by staging a slot-resident factor in %xmm1 (`movsd N(%rsp), %xmm1;
 * vfmadd231sd %xmm1, %xmm1, %xmmACC` for `d*d`), by swapping a register-homed
 * factor into src1 (IEEE multiplication is exactly commutative), or by
 * folding a slot or pool constant as the memory source. A factor that shares
 * the destination's home takes the 132-form, staging a slot-resident addend
 * in %xmm0.
 *
 * Every input is an integer-valued double/float and every product and sum is
 * exact, so fused and unfused evaluation agree bit-for-bit and any operand
 * mix-up (wrong register, stale %xmm1, accumulator clobbered by a preload)
 * changes the result. The distance sums have a closed form; `mixed` is
 * checked against a reference that keeps every value in memory.
 * Self-checking: prints the sums and returns non-zero on any mismatch.
 */
#include <stdio.h>

typedef struct { double x, y, z; } P;
typedef struct { float x, y, z; } Pf;

static P mk(int i) {
    P p = { i * 3.0, i * 5.0, i * 7.0 };
    return p;
}

static double d2(P a, P b) {
    double dx = a.x - b.x, dy = a.y - b.y, dz = a.z - b.z;
    return dx * dx + dy * dy + dz * dz;
}

/* Four particles at base..base+3: pair offsets 1,1,1,2,2,3 give
 * sum(delta^2) = 20, times 3^2 + 5^2 + 7^2 = 83 -> 1660 per group. */
static double group(int base) {
    P p[4];
    for (int k = 0; k < 4; k++)
        p[k] = mk(base + k);
    double t = 0;
    for (int i = 0; i < 4; i++)
        for (int j = i + 1; j < 4; j++)
            t += d2(p[i], p[j]);
    return t;
}

static Pf mkf(int i) {
    Pf p = { i * 3.0f, i * 5.0f, i * 7.0f };
    return p;
}

static float d2f(Pf a, Pf b) {
    float dx = a.x - b.x, dy = a.y - b.y, dz = a.z - b.z;
    return dx * dx + dy * dy + dz * dz;
}

static float groupf(int base) {
    Pf p[4];
    for (int k = 0; k < 4; k++)
        p[k] = mkf(base + k);
    float t = 0;
    for (int i = 0; i < 4; i++)
        for (int j = i + 1; j < 4; j++)
            t += d2f(p[i], p[j]);
    return t;
}

/* Constant and mixed-home factors with eight live accumulators: acc + v*0.5,
 * acc + v*w, acc + v*v, acc + 0.25*v. Exact for small integer v. */
__attribute__((noinline)) static double mixed(const double *v, int n) {
    double a0 = 0, a1 = 0, a2 = 0, a3 = 0, a4 = 0, a5 = 0, a6 = 0, a7 = 0;
    for (int i = 0; i + 8 <= n; i += 8) {
        a0 += v[i] * 0.5;
        a1 += v[i + 1] * v[i + 2];
        a2 += v[i + 2] * v[i + 2];
        a3 += 0.25 * v[i + 3];
        a4 += v[i + 4] * v[i + 5];
        a5 += v[i + 5] * 2.0;
        a6 += v[i + 6] * v[i + 7];
        a7 += v[i + 7] * v[i + 7];
    }
    return a0 + 2 * a1 + 3 * a2 + 4 * a3 + 5 * a4 + 6 * a5 + 7 * a6 + 8 * a7;
}

/* Reference: every intermediate goes through a volatile, so no FMA forms. */
static double mixed_ref(const double *v, int n) {
    volatile double a[8] = { 0 };
    for (int i = 0; i + 8 <= n; i += 8) {
        volatile double t;
        t = v[i] * 0.5; a[0] = a[0] + t;
        t = v[i + 1] * v[i + 2]; a[1] = a[1] + t;
        t = v[i + 2] * v[i + 2]; a[2] = a[2] + t;
        t = 0.25 * v[i + 3]; a[3] = a[3] + t;
        t = v[i + 4] * v[i + 5]; a[4] = a[4] + t;
        t = v[i + 5] * 2.0; a[5] = a[5] + t;
        t = v[i + 6] * v[i + 7]; a[6] = a[6] + t;
        t = v[i + 7] * v[i + 7]; a[7] = a[7] + t;
    }
    return a[0] + 2 * a[1] + 3 * a[2] + 4 * a[3] + 5 * a[4] + 6 * a[5] + 7 * a[6] + 8 * a[7];
}

/* SLP lane extraction leaves both lanes in stack slots, so x*y has two
 * slot-resident factors: lhs is staged in %xmm1, rhs is the memory source. */
__attribute__((noinline)) static double lanes(const double *a, const double *b,
                                              double *restrict out) {
    double x = a[0] - b[0], y = a[1] - b[1];
    out[0] = x;
    out[1] = y;
    return x * x + y * y + x * y;
}

/* Cross products: dz*dx where dx dies into the result's home (132-form with a
 * slot-resident addend). Pair offsets 1,1,1,2,2,3 give sum(delta^2) = 20,
 * times 3*5 + 5*7 + 7*3 = 71 -> 1420 per group. */
static double cr(P a, P b) {
    double dx = a.x - b.x, dy = a.y - b.y, dz = a.z - b.z;
    return dx * dy + dy * dz + dz * dx;
}

__attribute__((noinline)) static double groupx(int base) {
    P p[4];
    for (int k = 0; k < 4; k++)
        p[k] = mk(base + k);
    double t = 0;
    for (int i = 0; i < 4; i++)
        for (int j = i + 1; j < 4; j++)
            t += cr(p[i], p[j]);
    return t;
}

/* Loop-invariant multipliers and addends live across the call in stack
 * slots (every XMM register is caller-saved), so i*K + C contracts with the
 * converted i dying into the result's home: 132-form with the slot-resident
 * addend staged in %xmm0, which each call clobbers. 4a - 2b + c = 11i/32 + 6
 * exactly, so the sum over i = 1..199 is 8034.625. */
__attribute__((noinline)) static double comb(double a, double b, double c) {
    return a * 4.0 - b * 2.0 + c;
}

static volatile int affine_n = 200; /* opaque: keeps affine() from folding */

__attribute__((noinline)) static double affine(int n) {
    double t = 0;
    for (int i = 1; i < n; ++i)
        t += comb(i * .03125 + 1.0, i * .015625 + .5, i * .25 + 3.0);
    return t;
}

int main(void) {
    int bad = 0;
    double total = 0;
    for (int i = 0; i < 2000; i++)
        total += group(i);
    float totalf = 0;
    for (int i = 0; i < 40; i++)
        totalf += groupf(i);
    double v[64];
    for (int i = 0; i < 64; i++)
        v[i] = (double)(i % 13) - 6.0;
    double m = mixed(v, 64), mref = mixed_ref(v, 64);
    double totalx = 0;
    for (int i = 0; i < 2000; i++)
        totalx += groupx(i);
    int lanes_bad = 0;
    double b2[2] = { 0.5, -1.0 }, out[2];
    for (int i = -16; i <= 16; i++) {
        double a2[2] = { i, i + 1 };
        double x = i - 0.5, y = i + 2;
        double want = x * x + y * y + x * y;
        if (lanes(a2, b2, out) != want || out[0] != x || out[1] != y)
            lanes_bad = 1;
    }

    if (total != 1660.0 * 2000)
        bad |= 1;
    if (totalf != 1660.0f * 40)
        bad |= 2;
    if (m != mref)
        bad |= 4;
    if (totalx != 1420.0 * 2000)
        bad |= 8;
    if (lanes_bad)
        bad |= 16;
    if (affine(affine_n) != 8034.625)
        bad |= 32;
    printf("%.1f %.1f %.2f %.1f %d\n", total, (double)totalf, m, totalx, bad);
    return bad != 0;
}
