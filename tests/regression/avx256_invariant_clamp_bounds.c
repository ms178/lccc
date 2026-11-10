/* Loop-invariant clamp bounds must stay in registers across every iteration.
 *
 * A clamp whose `lo`/`hi` are written once and read by all eight stages used
 * to re-read both bounds from their stack slots on each stage, because nothing
 * recorded that the whole-register store had left the bounds resident in the
 * YMM registers that stored them.
 *
 * The fix replaces the FIRST VEX source -- the one position where AT&T syntax
 * allows a memory operand -- with a register. That is free of semantic change
 * only while the operand ORDER is preserved, and VMINPS/VMAXPS are emphatically
 * not commutative: when either operand is NaN the SECOND source wins, and on a
 * tie (including -0.0 vs +0.0) the second source wins too. Feeding src1 from a
 * register instead of memory must not reorder them.
 *
 * So this fixture pins both properties at once: the same two invariants are
 * read by every stage (register residency) over a lane set that exercises the
 * NaN, signed-zero, infinity and denormal corners (operand order).
 * Expectations come from a scalar model of the instruction, never from the
 * vector builtins, so a wrong rule cannot agree with itself. */
typedef float vf8 __attribute__((vector_size(32)));

/* AT&T `vminps A, B, dst` is Intel `VMINPS dst, B, A`: `A` is the r/m
 * (second) source and `B` the vvvv (first) source.  Both the NaN rule and the
 * tie rule return `A`, so `A` wins whenever the two are unordered or equal --
 * including -0.0 versus +0.0.  That is precisely why the operand slot being
 * rewritten here must not be reordered.
 *
 * Verified against hardware rather than taken from the SDM: lane 0 below is a
 * NaN input and must clamp to `hi`, and lane 2 is -0.0 and must clamp to +0.0. */
static float x86_min(float a, float b) {
    if (a != a || b != b) return a;
    if (a == b) return a;
    return b < a ? b : a;
}
static float x86_max(float a, float b) {
    if (a != a || b != b) return a;
    if (a == b) return a;
    return b > a ? b : a;
}

__attribute__((noinline)) void clamp8(vf8 *out, const vf8 *in, int n) {
    vf8 lo = {0, 0, 0, 0, 0, 0, 0, 0};
    vf8 hi = {1, 1, 1, 1, 1, 1, 1, 1};
    for (int i = 0; i < n; ++i)
        out[i] = __builtin_ia32_maxps256(__builtin_ia32_minps256(in[i], hi), lo);
}

int main(void) {
    /* -inf, -2, -0.0, denormal, 0.25, 1.0, quiet NaN, 0.5 */
    const unsigned bits[8] = {
        0xff800000u, 0xc0000000u, 0x80000000u, 0x00000001u,
        0x3e800000u, 0x3f800000u, 0x7fc00123u, 0x3f000000u,
    };
    const int n = 8;
    vf8 in[8], out[8];

    /* Every rotation, so each lane value meets both bounds and its neighbours
     * across the eight stages. */
    for (int r = 0; r < 8; ++r) {
        for (int i = 0; i < n; ++i) {
            union { vf8 v; unsigned u[8]; } x;
            for (int j = 0; j < 8; ++j) x.u[j] = bits[(i + j + r) & 7];
            in[i] = x.v;
        }
        clamp8(out, in, n);

        for (int i = 0; i < n; ++i) {
            union { vf8 v; unsigned u[8]; } got;
            got.v = out[i];
            for (int j = 0; j < 8; ++j) {
                union { unsigned u; float f; } w, g;
                w.u = bits[(i + j + r) & 7];
                g.u = got.u[j];
                /* Emitted as `vminps hi, x` then `vmaxps lo, min`. */
                float want = x86_max(0.0f, x86_min(1.0f, w.f));
                if (want != want) {
                    if (g.f == g.f) return 1 + r * 64 + i * 8 + j; /* not NaN */
                } else if (g.u != w.u && g.f != want) {
                    return 1 + r * 64 + i * 8 + j;
                }
            }
        }
    }
    return 0;
}
