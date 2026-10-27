/* FP-compare selects under legacy SSE (the SSE2 baseline): lccc lowers them
 * as a cmpss/cmpsd mask plus a bit select (comparison.rs
 * try_emit_fp_select_sse_mask) instead of a GPR cmov. Every predicate is
 * crossed with the arm shapes that take different emission paths (generic
 * XOR select, +0.0 true / false arm shortcuts, a -0.0 arm that must NOT take
 * the +0.0 shortcut, computed arms, swapped-operand predicates) and with a
 * call-spanning result whose destination has no XMM home. Inputs cover
 * +-0, +-inf, quiet NaNs with payloads, denormals and equal pairs; results
 * are hashed bit for bit, so a swapped arm, a lost NaN payload or a wrong
 * zero sign all change the output. */
#include <stdio.h>
#include <string.h>

#define NI __attribute__((noinline))

static volatile int sink_count;
NI static void sink(void) { sink_count++; }

#define SEL(name, T, OP)                                                     \
    NI T name##_gen(T a, T b, T t, T f) { return a OP b ? t : f; }           \
    NI T name##_zt(T a, T b, T f) { return a OP b ? (T)0 : f; }              \
    NI T name##_zf(T a, T b, T t) { return a OP b ? t : (T)0; }              \
    NI T name##_nz(T a, T b, T t) { return a OP b ? t : -(T)0; }             \
    NI T name##_ar(T a, T b) { return a OP b ? a + b : a * b; }              \
    NI T name##_sw(T a, T b) { return a OP b ? b : a; }                      \
    NI T name##_cs(T a, T b, T t, T f) {                                     \
        T r = a OP b ? t : f;                                                \
        sink();                                                              \
        return r * (T)2;                                                     \
    }

SEL(f_lt, float, <)
SEL(f_le, float, <=)
SEL(f_gt, float, >)
SEL(f_ge, float, >=)
SEL(f_eq, float, ==)
SEL(f_ne, float, !=)
SEL(d_lt, double, <)
SEL(d_le, double, <=)
SEL(d_gt, double, >)
SEL(d_ge, double, >=)
SEL(d_eq, double, ==)
SEL(d_ne, double, !=)

static float fbits(unsigned u) { float f; memcpy(&f, &u, 4); return f; }
static double dbits(unsigned long long u) { double d; memcpy(&d, &u, 8); return d; }
static float fbits_c(void) { return fbits(0x7fc00000u); }
static double dbits_c(void) { return dbits(0x7ff8000000000000ULL); }

static unsigned long long h = 1469598103934665603ULL;
static void mix(const void *p, size_t n) {
    const unsigned char *c = p;
    for (size_t i = 0; i < n; i++) h = (h ^ c[i]) * 1099511628211ULL;
}
static void mixf(float x) { mix(&x, sizeof x); }
static void mixd(double x) { mix(&x, sizeof x); }
/* Arithmetic on two NaNs: the payload follows operand order, which C leaves
 * unspecified, so computed arms hash NaN canonically. */
static void mixfc(float x) { if (x != x) x = fbits_c(); mixf(x); }
static void mixdc(double x) { if (x != x) x = dbits_c(); mixd(x); }


#define RUN(name, T, MIX, MIXC, V, N)                                              \
    for (int i = 0; i < N; i++)                                              \
        for (int j = 0; j < N; j++) {                                        \
            T a = V[i], b = V[j], t = V[(i + j + 1) % N], f = V[(3 * i + j + 2) % N]; \
            MIX(name##_gen(a, b, t, f));                                     \
            MIX(name##_zt(a, b, f));                                         \
            MIX(name##_zf(a, b, t));                                         \
            MIX(name##_nz(a, b, t));                                         \
            MIXC(name##_ar(a, b));                                            \
            MIX(name##_sw(a, b));                                            \
            MIX(name##_cs(a, b, t, f));                                      \
        }

int main(void) {
    float fv[12];
    double dv[12];
    const float fl[] = {0.0f, 1.0f, -1.0f, 2.5f, -2.5f, 1e-40f};
    const double dl[] = {0.0, 1.0, -1.0, 2.5, -2.5, 1e-310};
    for (int i = 0; i < 6; i++) { fv[i] = fl[i]; dv[i] = dl[i]; }
    fv[6] = fbits(0x80000000u); dv[6] = dbits(0x8000000000000000ULL);  /* -0 */
    fv[7] = fbits(0x7f800000u); dv[7] = dbits(0x7ff0000000000000ULL);  /* +inf */
    fv[8] = fbits(0xff800000u); dv[8] = dbits(0xfff0000000000000ULL);  /* -inf */
    fv[9] = fbits(0x7fc00123u); dv[9] = dbits(0x7ff8000000000123ULL);  /* qNaN */
    fv[10] = fbits(0xffc04560u); dv[10] = dbits(0xfff8000000004560ULL); /* -qNaN */
    fv[11] = 2.5f; dv[11] = 2.5;                                       /* equal pair */
    RUN(f_lt, float, mixf, mixfc, fv, 12)
    RUN(f_le, float, mixf, mixfc, fv, 12)
    RUN(f_gt, float, mixf, mixfc, fv, 12)
    RUN(f_ge, float, mixf, mixfc, fv, 12)
    RUN(f_eq, float, mixf, mixfc, fv, 12)
    RUN(f_ne, float, mixf, mixfc, fv, 12)
    RUN(d_lt, double, mixd, mixdc, dv, 12)
    RUN(d_le, double, mixd, mixdc, dv, 12)
    RUN(d_gt, double, mixd, mixdc, dv, 12)
    RUN(d_ge, double, mixd, mixdc, dv, 12)
    RUN(d_eq, double, mixd, mixdc, dv, 12)
    RUN(d_ne, double, mixd, mixdc, dv, 12)
    printf("%016llx %d\n", h, sink_count);
    return 0;
}
