/*
 * Loop-rotation correctness under register pressure.
 *
 * The `.env` sidecar sets BOTH `CCC_LOOP_ROTATE=1` and
 * `CCC_LOOP_ROTATE_IGNORE_PRESSURE=1`, i.e. the strictest configuration: the
 * pass is on and its Guard G profitability gate is disabled, so the
 * high-pressure loop below really is rotated. That is the configuration worth
 * pinning -- when Guard G fires the loop is simply left alone, which the rest
 * of the suite already covers, whereas a forced rotation of a 32-live-out
 * loop exercises the exit-phi machinery Guard G exists to cost.
 *
 * Root cause this guards (measured, x86-64 -O2): rotation makes the loop exit
 * reachable from two edges, so every loop-carried value live out of the loop
 * needs an exit phi merging its init with the body's last definition. With 32
 * such values on a 16-register target the allocator spilled inside the hot
 * loop (22 -> 47 stack moves) and the benchmark lost 24.9%. The transform was
 * never wrong, only unprofitable -- so this test pins the *semantics* and
 * `check_loop_rotate_pressure.sh` pins the *decision*.
 *
 * The reference is the same recurrence written as an in-place array sweep, so
 * it shares no code shape with the code under test. Note the updates are
 * Gauss-Seidel, not Jacobi: v[k] is read by later k in the same iteration
 * (v[30] += v[31]*v[0] uses the already-updated v[0]), which is what makes a
 * naive parallel-update reference disagree.
 */
#include <stdio.h>

#define NV 32
#define ITER 2000

static int reference(int n)
{
    int v[NV];
    for (int k = 0; k < NV; k++)
        v[k] = k + 1;
    for (int it = 0; it < n; it++)
        for (int k = 0; k < NV; k++)
            v[k] += v[(k + 1) % NV] * v[(k + 2) % NV];
    int x = 0;
    for (int k = 0; k < NV; k++)
        x ^= v[k];
    return x;
}

/* 32 loop-carried values, all 32 live out: the shape Guard G costs. */
static int many_live_out(int n)
{
    int a = 1, b = 2, c = 3, d = 4, e = 5, f = 6, g = 7, h = 8;
    int i = 9, j = 10, k = 11, l = 12, m = 13, o = 14, p = 15, q = 16;
    int r = 17, s = 18, t = 19, u = 20, v = 21, w = 22, x = 23, y = 24;
    int z0 = 25, z1 = 26, z2 = 27, z3 = 28, z4 = 29, z5 = 30, z6 = 31, z7 = 32;

    for (int iter = 0; iter < n; iter++) {
        a += b * c; b += c * d; c += d * e; d += e * f;
        e += f * g; f += g * h; g += h * i; h += i * j;
        i += j * k; j += k * l; k += l * m; l += m * o;
        m += o * p; o += p * q; p += q * r; q += r * s;
        r += s * t; s += t * u; t += u * v; u += v * w;
        v += w * x; w += x * y; x += y * z0; y += z0 * z1;
        z0 += z1 * z2; z1 += z2 * z3; z2 += z3 * z4; z3 += z4 * z5;
        z4 += z5 * z6; z5 += z6 * z7; z6 += z7 * a; z7 += a * b;
    }
    return a ^ b ^ c ^ d ^ e ^ f ^ g ^ h ^ i ^ j ^ k ^ l ^
           m ^ o ^ p ^ q ^ r ^ s ^ t ^ u ^ v ^ w ^ x ^ y ^
           z0 ^ z1 ^ z2 ^ z3 ^ z4 ^ z5 ^ z6 ^ z7;
}

/* One live-out value: rotation stays profitable, so it must still rotate
 * (check_loop_rotate_pressure.sh asserts that) and stay correct. */
static int one_live_out(int n)
{
    int a = 1, b = 2, c = 3, d = 4;
    for (int iter = 0; iter < n; iter++) {
        a += b * c;
        b += c * d;
        c += d * a;
        d += a * b;
    }
    return a;
}

static int reference_small(int n)
{
    int v[4] = { 1, 2, 3, 4 };
    for (int it = 0; it < n; it++)
        for (int k = 0; k < 4; k++)
            v[k] += v[(k + 1) % 4] * v[(k + 2) % 4];
    return v[0];
}

/* Live-out through a terminator: the IV escapes via the exit CondBranch and
 * is then returned, so the exit phi's incoming comes from a branch operand.
 * Guard G counts terminator operands for exactly this reason. */
static int live_out_via_terminator(int n)
{
    int i = 0;
    int acc = 7;
    while (i < n) {
        acc ^= (i * 2654435761u) >> 7;
        i++;
    }
    return acc + i;
}

static int reference_term(int n)
{
    int i = 0;
    int acc = 7;
    while (i != n) {
        acc = acc ^ (int)(((unsigned)i * 2654435761u) >> 7);
        i = i + 1;
    }
    return acc + i;
}

int main(void)
{
    long fail = 0;

    if (many_live_out(ITER) != reference(ITER))
        fail++;
    if (many_live_out(0) != reference(0))
        fail++;
    if (many_live_out(1) != reference(1))
        fail++;
    if (one_live_out(ITER) != reference_small(ITER))
        fail++;
    if (one_live_out(0) != reference_small(0))
        fail++;
    if (live_out_via_terminator(64) != reference_term(64))
        fail++;
    if (live_out_via_terminator(0) != reference_term(0))
        fail++;

    printf("fail=%ld\n", fail);
    return fail != 0;
}
