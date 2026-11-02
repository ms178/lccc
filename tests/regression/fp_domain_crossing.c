// FP domain-crossing regression test — catches bugs from XMM↔GPR value-flow
// optimizations.  Covers: chained FP binops, FP accumulators in loops,
// int→float casts feeding FP chains, FP in nested loops with struct access,
// FP values live across function calls (sqrt).
// Expected output: all lines "OK"
#include <stdio.h>
#include <math.h>
#include <string.h>

#define CHECK(tag, got, expect, tol) do { \
    double _d = (got) - (expect); \
    if (_d < 0) _d = -_d; \
    printf("%s: %s (got=%.15e expect=%.15e diff=%.3e)\n", \
           tag, _d <= (tol) ? "OK" : "FAIL", (double)(got), (double)(expect), _d); \
} while(0)

// Test 1: simple chained FP ops (add, mul, sub, div)
static double fp_chain(double a, double b, double c) {
    double x = a + b;    // FP add
    double y = x * c;    // FP mul
    double z = y - a;    // FP sub
    double w = z / b;    // FP div
    return w;
}

// Test 2: FP accumulator in a loop (reduction pattern)
static double fp_accum(int n) {
    double sum = 0.0;
    for (int i = 1; i <= n; i++) {
        sum += 1.0 / (double)i;
    }
    return sum;
}

// Test 3: FP values from int→float cast flowing into FP ops
static double int_to_fp_chain(int a, int b, int c) {
    double da = (double)a;
    double db = (double)b;
    double dc = (double)c;
    return da * db + dc;
}

// Test 4: FP in nested loop with struct-like memory access
typedef struct { double x, y, z, vx, vy, vz, mass; } Body;
static Body test_bodies[3] = {
    { 1.0, 2.0, 3.0, 0.1, 0.2, 0.3, 100.0 },
    { 4.0, 5.0, 6.0, 0.4, 0.5, 0.6, 200.0 },
    { 7.0, 8.0, 9.0, 0.7, 0.8, 0.9, 300.0 },
};

static double nested_struct_fp(int n) {
    double e = 0.0;
    for (int step = 0; step < n; step++) {
        for (int i = 0; i < 3; i++) {
            for (int j = i + 1; j < 3; j++) {
                double dx = test_bodies[i].x - test_bodies[j].x;
                double dy = test_bodies[i].y - test_bodies[j].y;
                double dz = test_bodies[i].z - test_bodies[j].z;
                double d2 = dx*dx + dy*dy + dz*dz;
                double mag = 0.01 / (d2 * sqrt(d2));
                test_bodies[i].vx -= dx * test_bodies[j].mass * mag;
                test_bodies[i].vy -= dy * test_bodies[j].mass * mag;
                test_bodies[i].vz -= dz * test_bodies[j].mass * mag;
                test_bodies[j].vx += dx * test_bodies[i].mass * mag;
                test_bodies[j].vy += dy * test_bodies[i].mass * mag;
                test_bodies[j].vz += dz * test_bodies[i].mass * mag;
            }
        }
        for (int i = 0; i < 3; i++) {
            test_bodies[i].x += 0.01 * test_bodies[i].vx;
            test_bodies[i].y += 0.01 * test_bodies[i].vy;
            test_bodies[i].z += 0.01 * test_bodies[i].vz;
        }
    }
    for (int i = 0; i < 3; i++) {
        e += test_bodies[i].x + test_bodies[i].vx;
    }
    return e;
}

// Test 5: FP values live across sqrt call (call-spanning XMM issue)
static double fp_across_call(double a, double b, double c) {
    double before = a * b + c;
    double s = sqrt(before);
    double after = s * a - b;
    return after;
}

// Test 6: FP phi (loop-carried FP value)
static double fp_phi_loop(double start, int n) {
    double val = start;
    for (int i = 0; i < n; i++) {
        val = val * 1.0000001 + 0.0000001;
    }
    return val;
}

// Test 7: FP select/conditional
static double fp_select(double a, double b, int cond) {
    double x = cond ? a : b;
    return x * 2.0;
}

// Test 8: FP comparison
static double fp_cmp(double a, double b) {
    if (a > b) return a - b;
    return b - a;
}

int main(void) {
    // Test 1: fp_chain(1.5, 2.5, 3.0) → x=4, y=12, z=10.5, w=4.2
    CHECK("fp_chain", fp_chain(1.5, 2.5, 3.0), 4.2, 1e-12);
    
    // Test 2: H_n for n=1000 — reference: 7.485470860550345
    CHECK("fp_accum", fp_accum(1000), 7.485470860550345, 1e-10);
    
    // Test 3: 3 * 4 + 5 = 17
    CHECK("int_to_fp", int_to_fp_chain(3, 4, 5), 17.0, 1e-12);
    
    // Test 4: nested struct FP
    double ns = nested_struct_fp(100);
    printf("nested_struct: %.15e\n", ns);
    
    // Test 5: FP across sqrt call
    CHECK("fp_across_call", fp_across_call(3.0, 4.0, 5.0), sqrt(17.0)*3.0 - 4.0, 1e-12);
    
    // Test 6: FP phi loop
    double phi = fp_phi_loop(1.0, 1000000);
    printf("fp_phi: %.15e\n", phi);
    
    // Test 7: FP select
    CHECK("fp_select_t", fp_select(3.0, 4.0, 1), 6.0, 1e-12);
    CHECK("fp_select_f", fp_select(3.0, 4.0, 0), 8.0, 1e-12);
    
    // Test 8: FP comparison
    CHECK("fp_cmp_a", fp_cmp(5.0, 3.0), 2.0, 1e-12);
    CHECK("fp_cmp_b", fp_cmp(3.0, 5.0), 2.0, 1e-12);
    
    return 0;
}
