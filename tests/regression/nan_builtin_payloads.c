/* NaN / infinity builtins with payloads, in every binary format, both as
 * static initializers (folded by the static-init path) and as automatic
 * initializers (folded in function bodies), plus `typeof` of builtin calls
 * and the GCC `q` (_Float128) builtins.  Only value bytes are printed, never
 * padding (x87 long double has 10 value bytes in a 12/16-byte object).
 *
 * Regressions pinned here:
 * - __builtin_nanl wrote big-endian payload bytes into the little-endian
 *   x87 carrier (a second, divergent NaN parser in the lowerer);
 * - __builtin_nans/nansf/nansl were missing from the builtin table and so
 *   were implicitly-declared int functions in function bodies;
 * - static _Float128 NaN/Inf initializers were rejected;
 * - typeof(__builtin_nan("")) fell back to int;
 * - the `q` builtins (nanq, infq, huge_valq, fabsq, copysignq) were
 *   implicitly-declared int functions.
 * (fabsq/copysignq are not constant initializers in GCC either, so they
 * are exercised in function bodies only.) */
#include <stdio.h>
#include <string.h>

static void dump(const char *name, const void *p, size_t n) {
    const unsigned char *b = p;
    printf("%s:", name);
    while (n--)
        printf("%02x", b[n]); /* little-endian: most significant byte first */
    printf("\n");
}

#if defined(__x86_64__) || defined(__i386__)
#define LD_BYTES 10
#else
#define LD_BYTES sizeof(long double)
#endif

#define D(name, v) dump(name, &(v), sizeof(v))
#define LD(name, v) dump(name, &(v), LD_BYTES)

static const double s_nan7 = __builtin_nan("7");
static const double s_nanhex = __builtin_nan("0x8000000000001");
static const double s_nans = __builtin_nans("0x10000000000000");
static const double s_nans0 = __builtin_nans("");
static const float s_nanf = __builtin_nanf("0x3ff");
static const float s_nansf = __builtin_nansf("0x800000");
static const float s_nansf1 = __builtin_nansf("1");
static const long double s_nanl = __builtin_nanl("7");
static const long double s_nansl0 = __builtin_nansl("");
static const long double s_nansl5 = __builtin_nansl("5");
static const double s_inf = __builtin_inf();
static const float s_inff = __builtin_inff();
static const long double s_infl = __builtin_infl();
static const double s_hv = __builtin_huge_val();
static const _Float128 s_q7 = __builtin_nanf128("7");
static const _Float128 s_qs0 = __builtin_nansf128("");
static const _Float128 s_qs5 = __builtin_nansf128("5");
static const _Float128 s_qinf = __builtin_inff128();
static const _Float128 s_nanq = __builtin_nanq("3");
static const _Float128 s_nansq = __builtin_nansq("");
static const _Float128 s_infq = __builtin_infq();
static const _Float128 s_hvq = __builtin_huge_valq();

/* Static array elements go through the aggregate initializer path. */
static const double s_arr[3] = {__builtin_nan("1"), __builtin_inf(), -__builtin_inf()};
static const _Float128 s_qarr[2] = {__builtin_nanq("2"), -__builtin_infq()};

static void statics(void) {
    D("s_nan7", s_nan7);
    D("s_nanhex", s_nanhex);
    D("s_nans", s_nans);
    D("s_nans0", s_nans0);
    D("s_nanf", s_nanf);
    D("s_nansf", s_nansf);
    D("s_nansf1", s_nansf1);
    LD("s_nanl", s_nanl);
    LD("s_nansl0", s_nansl0);
    LD("s_nansl5", s_nansl5);
    D("s_inf", s_inf);
    D("s_inff", s_inff);
    LD("s_infl", s_infl);
    D("s_hv", s_hv);
    D("s_q7", s_q7);
    D("s_qs0", s_qs0);
    D("s_qs5", s_qs5);
    D("s_qinf", s_qinf);
    D("s_nanq", s_nanq);
    D("s_nansq", s_nansq);
    D("s_infq", s_infq);
    D("s_hvq", s_hvq);
    for (int i = 0; i < 3; i++)
        D("s_arr", s_arr[i]);
    for (int i = 0; i < 2; i++)
        D("s_qarr", s_qarr[i]);
}

/* Automatic objects: the function-body constant folder.  Stored straight
 * into memory (memcpy), so no FP register round trip can quiet a sNaN.
 * (GCC -O0 on i386 does route them through x87 fld/fstp, which quiets
 * signaling NaNs; the test runs at -O2, where GCC stores the bits.) */
static void autos(void) {
    unsigned char buf[16];
#define A(name, T, expr)                  \
    do {                                  \
        T v_ = (expr);                    \
        memcpy(buf, &v_, sizeof v_);      \
        dump(name, buf, sizeof v_);       \
    } while (0)
#define AL(name, expr)                    \
    do {                                  \
        long double v_ = (expr);          \
        memcpy(buf, &v_, LD_BYTES);       \
        dump(name, buf, LD_BYTES);        \
    } while (0)
    A("a_nan7", double, __builtin_nan("7"));
    A("a_nans", double, __builtin_nans("0x10000000000000"));
    A("a_nansf", float, __builtin_nansf("0x800000"));
    AL("a_nanl", __builtin_nanl("7"));
    AL("a_nansl0", __builtin_nansl(""));
    AL("a_nansl5", __builtin_nansl("5"));
    A("a_q7", _Float128, __builtin_nanf128("7"));
    A("a_qs5", _Float128, __builtin_nansf128("5"));
    A("a_nanq", _Float128, __builtin_nanq("3"));
    A("a_infq", _Float128, __builtin_infq());
    A("a_fabsq", _Float128, __builtin_fabsq(-2.5Q));
    A("a_cpsq", _Float128, __builtin_copysignq(3.0Q, -2.5Q));
}

/* typeof of an undeclared builtin call names the builtin's return type. */
static void types(void) {
    __typeof__(__builtin_nan("")) d = 1.5;
    __typeof__(__builtin_nanf("")) f = 2.5f;
    __typeof__(__builtin_nanl("")) l = 3.5L;
    __typeof__(__builtin_nanf128("")) q = 4.5Q;
    __typeof__(__builtin_infq()) iq = 5.5Q;
    __typeof__(__builtin_fabsq(1.0Q)) fq = 6.5Q;
    printf("typeof sizes %d %d %d %d %d %d\n", (int)sizeof d, (int)sizeof f,
           (int)sizeof l, (int)sizeof q, (int)sizeof iq, (int)sizeof fq);
    printf("typeof values %.1f %.1f %.1Lf %.1f %.1f %.1f\n", d, (double)f, l,
           (double)q, (double)iq, (double)fq);
    printf("generic %d %d %d\n",
           _Generic(__builtin_nans(""), double: 1, default: 0),
           _Generic(__builtin_nansf(""), float: 1, default: 0),
           _Generic(__builtin_nanq(""), _Float128: 1, default: 0));
}

/* Classification at run time (isnan/isinf on the folded values). */
static void classify(void) {
    volatile double n = __builtin_nan("");
    volatile _Float128 qn = __builtin_nanq("");
    volatile _Float128 qi = __builtin_infq();
    printf("classify %d %d %d %d %d\n", n != n, __builtin_isnan(n),
           qn != qn, qi > 1.0e4000Q, -qi < -1.0e4000Q);
}

int main(void) {
    statics();
    autos();
    types();
    classify();
    return 0;
}
