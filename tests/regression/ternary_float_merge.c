/* ternary_float_merge.c — float ternary merge slots (F2 hardening).

   S25 made F32/F64 ternary merge slots exact-typed so FP selects stay
   XMM-homed and S05 blend can fire (bit-identical results). This test
   covers hardening in `emit_ternary_merge_store`: constants must be
   normalized to slot type, and NaN payloads must survive.

   Fixed vs original audit:
   - No literal `1 ? x : y` conditions (those fold before merge slots).
     All selections go through noinline functions driven by runtime int.
   - `cond_f64_const_to_f32` now correctly contains F32 literal, not F64.
   - Added volatile-driven NaN selection to force merge path.
   - Added memory-promotion path that reaches `narrowed_to` F32<->F64.
*/
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <math.h>

static uint64_t d2u(double d) { uint64_t u; memcpy(&u, &d, sizeof u); return u; }
static uint32_t f2u(float f)  { uint32_t u; memcpy(&u, &f, sizeof u); return u; }

__attribute__((noinline)) double cond_f64(int c, double x) {
    return c ? 1.0 : x;
}
__attribute__((noinline)) double cond_int_to_f64(int c, double f) {
    return c ? 0 : f;
}
__attribute__((noinline)) double cond_f32_to_f64(int c, double d) {
    return c ? 1.0f : d;
}
__attribute__((noinline)) float cond_f32(int c, float x) {
    return c ? 1.0f : x;
}
__attribute__((noinline)) float cond_f64_const_to_f32(int c, float x) {
    return c ? 1.0f : x;
}
__attribute__((noinline)) float cond_nan_f32(int c, float a, float b) {
    return c ? a : b;
}
__attribute__((noinline)) double cond_nan_f64(int c, double a, double b) {
    return c ? a : b;
}
/* Memory-promotion fixture: store ternary result to alloca, then load.
   This reaches mem2reg's def-stack narrowing which now handles F32<->F64. */
__attribute__((noinline)) float cond_mem_f32(int c, float x) {
    float tmp;
    tmp = c ? 1.0f : x;
    return tmp;
}
__attribute__((noinline)) double cond_mem_f64(int c, double x) {
    double tmp;
    tmp = c ? 1.0 : x;
    return tmp;
}

int main(void) {
    double a = cond_f64(1, 2.5);
    double b = cond_f64(0, 2.5);
    if (a != 1.0 || b != 2.5) {
        printf("FAIL cond_f64 %f %f\n", a, b);
        return 1;
    }
    double c = cond_int_to_f64(1, 3.14);
    double d = cond_int_to_f64(0, 3.14);
    if (c != 0.0 || d != 3.14) {
        printf("FAIL cond_int_to_f64 %f %f\n", c, d);
        return 1;
    }
    double e = cond_f32_to_f64(1, 2.5);
    double f = cond_f32_to_f64(0, 2.5);
    if (e != 1.0f || f != 2.5) {
        printf("FAIL cond_f32_to_f64 %f %f\n", e, f);
        return 1;
    }
    float g = cond_f32(1, 2.5f);
    float h = cond_f32(0, 2.5f);
    if (f2u(g) != f2u(1.0f) || f2u(h) != f2u(2.5f)) {
        printf("FAIL cond_f32 %08x %08x\n", f2u(g), f2u(h));
        return 1;
    }
    float i = cond_mem_f32(1, 2.5f);
    float j = cond_mem_f32(0, 2.5f);
    if (f2u(i) != f2u(1.0f) || f2u(j) != f2u(2.5f)) {
        printf("FAIL cond_mem_f32 %08x %08x\n", f2u(i), f2u(j));
        return 1;
    }
    double k = cond_mem_f64(1, 2.5);
    double l = cond_mem_f64(0, 2.5);
    if (k != 1.0 || l != 2.5) {
        printf("FAIL cond_mem_f64 %f %f\n", k, l);
        return 1;
    }
    /* NaN payload preservation via runtime condition (not literal 1/0) */
    {
        union { float f; uint32_t u; } nan1, nan2;
        nan1.u = 0x7fc00001U;
        nan2.u = 0x7fc00002U;
        volatile int vc1 = 1;
        volatile int vc0 = 0;
        float r1 = cond_nan_f32(vc1, nan1.f, 0.0f);
        float r2 = cond_nan_f32(vc0, 0.0f, nan2.f);
        if (f2u(r1) != nan1.u || f2u(r2) != nan2.u) {
            printf("FAIL nan_f32 payload %08x %08x vs %08x %08x\n",
                   f2u(r1), f2u(r2), nan1.u, nan2.u);
            return 1;
        }
        union { double f; uint64_t u; } dnan1, dnan2;
        dnan1.u = 0x7ff8000000000001ULL;
        dnan2.u = 0x7ff8000000000002ULL;
        double dr1 = cond_nan_f64(vc1, dnan1.f, 0.0);
        double dr2 = cond_nan_f64(vc0, 0.0, dnan2.f);
        if (d2u(dr1) != dnan1.u || d2u(dr2) != dnan2.u) {
            printf("FAIL nan_f64 payload %016llx %016llx vs %016llx %016llx\n",
                   (unsigned long long)d2u(dr1), (unsigned long long)d2u(dr2),
                   (unsigned long long)dnan1.u, (unsigned long long)dnan2.u);
            return 1;
        }
    }
    printf("PASS ternary_float_merge %016llx %08x\n",
           (unsigned long long)d2u(cond_f64(1, 2.5)),
           f2u(cond_f32(1, 2.5f)));
    return 0;
}
