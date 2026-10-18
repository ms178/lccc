// AUD-VABI-4: one-lane floating-point ext_vector_type returns travel in the
// low half of %xmm0 on x86-64 (matching scalar f32/f64 returns, Clang, and
// GCC).  Earlier LCCC builds carried a ONE-element double vector in %rax via
// the INTEGER eightbyte class, which broke lccc->lccc at -O1 (the callee
// materialised a `vaddsd` into %xmm2 and emitted no movq into %rax) and
// diverged from the oracles' same-compiler codegen.  This gate exercises the
// return register at -O0/-O1/-O2, chain-calls a returned vector into another
// function (which forced an IR-lowering fix: packed-vector results had to be
// spilled to a temp before being passed as a struct-by-value SSE argument),
// and verifies elementwise arithmetic returns land in %xmm0.
#include <stdio.h>
#include <string.h>

typedef double double1 __attribute__((ext_vector_type(1)));
typedef float  float1  __attribute__((ext_vector_type(1)));

__attribute__((noinline)) double1 did(double1 a) { return a; }
__attribute__((noinline)) double1 dadd(double1 a, double1 b) { return a + b; }
__attribute__((noinline)) double1 dmul(double1 a, double1 b) { return a * b; }
__attribute__((noinline)) double1 dsub(double1 a, double1 b) { return a - b; }

__attribute__((noinline)) float1 fid(float1 a) { return a; }
__attribute__((noinline)) float1 fadd(float1 a, float1 b) { return a + b; }
__attribute__((noinline)) float1 fmul(float1 a, float1 b) { return a * b; }

static double as_d(double1 v) { double r; memcpy(&r, &v, sizeof r); return r; }
static float  as_f(float1 v)  { float r;  memcpy(&r, &v, sizeof r); return r; }

static int fails = 0;
#define DCK(expr, want) do { \
    double got = as_d(expr); \
    if (got != (want)) { printf("FAIL %s:%d: got %g want %g\n", __FILE__, __LINE__, got, (double)(want)); fails++; } \
} while (0)
#define FCK(expr, want) do { \
    float got = as_f(expr); \
    if (got != (want)) { printf("FAIL %s:%d: got %g want %g\n", __FILE__, __LINE__, (double)got, (double)(float)(want)); fails++; } \
} while (0)

int main(void) {
    double1 x = (double1){3.25};
    double1 y = (double1){1.5};
    DCK(did(x), 3.25);
    DCK(dadd(x, y), 4.75);
    DCK(dmul(x, y), 4.875);
    DCK(dsub(x, y), 1.75);
    // Chain pass: returned vector is forwarded as a by-value SSE arg to a
    // second function; this requires the caller to spill the packed xmm0
    // result to a temp and reload it from that address, otherwise the bit
    // pattern in xmm0 is reinterpreted as a pointer and the second load
    // segfaults.
    DCK(dadd(did(x), dadd(y, y)), 6.25);

    float1 fx = (float1){2.5f};
    float1 fy = (float1){4.0f};
    FCK(fid(fx), 2.5f);
    FCK(fadd(fx, fy), 6.5f);
    FCK(fmul(fx, fy), 10.0f);

    printf("fails=%d\n", fails);
    return fails != 0;
}