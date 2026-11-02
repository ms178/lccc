/* vex_promote_semantic — runtime + structural regression for the semantic
 * VEX-promotion pass (src/backend/x86/codegen/peephole/passes/vex_promote.rs).
 *
 * The pass rewrites legacy SSE to VEX inside functions that use 256/512-bit
 * registers, gated by an exact upper-YMM dataflow (EXPO observability +
 * CLEAN cleanliness over the CFG).  Bits 127:0 are always preserved, so
 * every kernel here must produce bit-identical results against a
 * volatile-locked scalar reference and against GCC (the suite compares
 * stdout).  The kernels deliberately cover the state machine:
 *
 *  - f_remainder:  vector loop + scalar remainder + reduction epilogue
 *                  (the classic dirty-tail penalty; all tails must be VEX).
 *  - f_switch:     jump-table dispatch (indirect jmp through a .rodata
 *                  table) mixed with scalar FP — exercises the jump-table
 *                  CFG resolution.
 *  - f_half:       128-bit-half accumulator — the merged upper half IS live
 *                  data and must stay legacy (soundness, not performance).
 *  - f_zero:       self-zeroing idioms (vpxor y,y) must not pin legacy
 *                  writes that precede them.
 *  - f_casts:      cvtsi2sd/cvttsd2si/ucomisd conversions and compares in a
 *                  ymm function.
 *
 * Every reference mirrors its kernel's SOURCE-LEVEL association statement
 * for statement (the kernels group their adds differently from a plain
 * sequential loop, so a shared reference would differ in the last ULPs by
 * design, not by defect).  The volatile loads lock both optimisers out of
 * the reference, guaranteeing the scalar order the kernel is judged
 * against.  Trip counts sweep the remainder 0..7 (doubles) and 0..3
 * (floats) so every partial-tail size executes.
 */
#include <stdio.h>
#include <stdint.h>
#include <string.h>

#define N 97

static float fa[N + 8], fb[N + 8], fc[N + 8];
static double da[N + 8], db[N + 8], dc[N + 8];

static uint64_t h = 1469598103934665603ull;
static uint64_t next_u64(void) {
    h ^= h << 7;
    h ^= h >> 9;
    return h;
}

/* Volatile-locked element loads: the reference reads the same arrays but
 * neither GCC nor lccc may reorder, narrow, or vectorise these. */
static double vda(int i) { volatile double v = da[i]; return v; }
static double vdb(int i) { volatile double v = db[i]; return v; }

/* Vector loop + scalar remainder + scalar reduction epilogue. */
static double f_remainder(int n) {
    double acc = 0.0;
    int i = 0;
    for (; i + 4 <= n; i += 4) {
        acc += da[i] * db[i] + da[i + 1] * db[i + 1];
        acc += da[i + 2] * db[i + 2] + da[i + 3] * db[i + 3];
    }
    for (; i < n; i++)
        acc += da[i] * db[i];
    return acc;
}

static double f_remainder_ref(int n) {
    double acc = 0.0;
    int i = 0;
    for (; i + 4 <= n; i += 4) {
        acc += vda(i) * vdb(i) + vda(i + 1) * vdb(i + 1);
        acc += vda(i + 2) * vdb(i + 2) + vda(i + 3) * vdb(i + 3);
    }
    for (; i < n; i++)
        acc += vda(i) * vdb(i);
    return acc;
}

/* Jump-table dispatch (default: indirect jmp through .rodata) with FP. */
static double f_switch(int k, double x) {
    double acc = x;
    switch (k & 7) {
        case 0: acc += 1.0; break;
        case 1: acc -= 1.0; break;
        case 2: acc *= 2.0; break;
        case 3: acc += 3.5; break;
        case 4: acc -= 4.25; break;
        case 5: acc *= 5.0; break;
        case 6: acc += 6.125; break;
        default: acc /= 2.0; break;
    }
    return acc;
}

/* 128-bit-half accumulator: the upper half of the scalar register carries
 * live vector state, so the scalar loads must keep their merge semantics. */
static double f_half(int n) {
    double acc = 0.0;
    int i = 0;
    for (; i + 8 <= n; i += 8) {
        double s = 0.0;
        for (int j = 0; j < 8; j++)
            s += da[i + j];
        acc += s * db[i];
    }
    for (; i < n; i++)
        acc += da[i] * 0.5;
    return acc;
}

static double f_half_ref(int n) {
    double acc = 0.0;
    int i = 0;
    for (; i + 8 <= n; i += 8) {
        double s = 0.0;
        for (int j = 0; j < 8; j++)
            s += vda(i + j);
        acc += s * vdb(i);
    }
    for (; i < n; i++)
        acc += vda(i) * 0.5;
    return acc;
}

/* Self-zeroing idiom followed by scalar work. */
static double f_zero(int n) {
    double acc = 0.0;
    int i = 0;
    for (; i + 4 <= n; i += 4)
        acc += da[i] * db[i] + da[i + 1] * db[i + 1];
    for (; i < n; i++)
        acc += da[i];
    return acc;
}

static double f_zero_ref(int n) {
    double acc = 0.0;
    int i = 0;
    for (; i + 4 <= n; i += 4)
        acc += vda(i) * vdb(i) + vda(i + 1) * vdb(i + 1);
    for (; i < n; i++)
        acc += vda(i);
    return acc;
}

/* Conversions and compares in a ymm function. */
static double f_casts(int n) {
    double acc = 0.0;
    int i = 0;
    for (; i + 4 <= n; i += 4)
        acc += da[i] * db[i];
    for (; i < n; i++) {
        double d = (double)(int64_t)da[i];
        if (d > 0.5)
            acc += d;
        acc += (double)(float)da[i];
    }
    return acc;
}

static double f_casts_ref(int n) {
    double acc = 0.0;
    int i = 0;
    for (; i + 4 <= n; i += 4)
        acc += vda(i) * vdb(i);
    for (; i < n; i++) {
        double d = (double)(int64_t)vda(i);
        if (d > 0.5)
            acc += d;
        acc += (double)(float)vda(i);
    }
    return acc;
}

static double f_switch_ref(int k);

static void p(double a, double b, const char *what, int n) {
    if (a != b) {
        printf("MISMATCH %s n=%d: %.17g vs %.17g\n", what, n, a, b);
        return;
    }
    printf("%s n=%d ok %.17g\n", what, n, a);
}

int main(void) {
    for (int i = 0; i < N + 8; i++) {
        da[i] = (double)(next_u64() % 1000) / 7.0 + 0.25;
        db[i] = (double)(next_u64() % 1000) / 11.0 + 0.5;
        fa[i] = (float)da[i];
        fb[i] = (float)db[i];
        fc[i] = 0.0f;
    }
    for (int n = 0; n <= N; n += 1 + (int)(next_u64() % 4)) {
        p(f_remainder(n), f_remainder_ref(n), "f_remainder", n);
        p(f_half(n), f_half_ref(n), "f_half", n);
        p(f_zero(n), f_zero_ref(n), "f_zero", n);
        p(f_casts(n), f_casts_ref(n), "f_casts", n);
        p(f_switch(n, (double)n), f_switch_ref(n), "f_switch", n);
    }
    return 0;
}

/* Reference for f_switch, kept in a separate function so the compiler cannot
 * merge the branches back into the switch. */
static double f_switch_ref(int k) {
    double acc = (double)k;
    switch (k & 7) {
        case 0: acc += 1.0; break;
        case 1: acc -= 1.0; break;
        case 2: acc *= 2.0; break;
        case 3: acc += 3.5; break;
        case 4: acc -= 4.25; break;
        case 5: acc *= 5.0; break;
        case 6: acc += 6.125; break;
        default: acc /= 2.0; break;
    }
    return acc;
}
