/* Conversions of a lowered `int` arithmetic value must start from the width
 * the value actually has (I32), not the widened LP64 storage type that
 * get_expr_type reports for int-op-literal (I64): with I64 as the source,
 * `i + 1 == -1` reached cvtsi2sdq / the 64-bit consumer zero-extended, i.e.
 * as 4294967295. Every initializer, splat and conversion path that lowers
 * the expression and then converts it is exercised with negative results.
 * Self-checking against values computed through plain declarations (the
 * path that always used the value width); returns non-zero on mismatch.
 */
#include <stdio.h>

typedef double v2d __attribute__((vector_size(16)));
typedef float v4f __attribute__((vector_size(16)));
typedef long long v2l __attribute__((vector_size(16)));

struct S { double a, b; };
struct V { int tag; v2d v; };
struct L { long long q; unsigned long long u; };

static int fails;

static void check(const char *what, double got, double want) {
    if (got != want) {
        printf("FAIL %s: got %.17g want %.17g\n", what, got, want);
        fails++;
    }
}

static void checkl(const char *what, long long got, long long want) {
    if (got != want) {
        printf("FAIL %s: got %lld want %lld\n", what, got, want);
        fails++;
    }
}

__attribute__((noinline)) static double ret_d(int i) { return i - 3; }
__attribute__((noinline)) static long long ret_ll(int i) { return i - 3; }
__attribute__((noinline)) static float ret_f(int i) { return i * 2 + 1; }
__attribute__((noinline)) static double _Complex ret_c(int i) { return i - 3; }
typedef signed char v4qi __attribute__((vector_size(4)));

__attribute__((noinline)) static void run(int i) {
    double want = i - 3;            /* reference: plain declaration */
    double want1 = i + 1;
    long long wantl = i - 3;

    double a[3] = { i, i + 1, i - 3 };
    check("array[1]", a[1], want1);
    check("array[2]", a[2], want);

    float f[2] = { 0, i - 3 };
    check("float array", f[1], (float)want);

    long double ld[2] = { 0, i - 3 };
    check("long double array", (double)ld[1], want);

    long long q[2] = { i - 3, i * 4 };
    checkl("i64 array", q[0], wantl);
    checkl("i64 array mul", q[1], (long long)(i * 4));

    unsigned long long uq[1] = { i - 3 };
    checkl("u64 array", (long long)uq[0], wantl);

    _Bool b[2] = { i - i, i + 1 };
    checkl("bool array", b[0] + 2 * b[1], i + 1 != 0 ? 2 : 0);

    double g = { i - 3 };
    check("braced scalar", g, want);

    long long gl = { i - 3 };
    checkl("braced i64 scalar", gl, wantl);

    struct S s = { i, i - 3 };
    check("struct member", s.b, want);

    struct L l = { i - 3, i - 3 };
    checkl("struct i64", l.q, wantl);
    checkl("struct u64", (long long)l.u, wantl);

    struct S sd = { .b = i - 3 };
    check("designated member", sd.b, want);

    struct S sa[2] = { { 0, 0 }, { i - 3, i + 1 } };
    check("array of structs", sa[1].a, want);
    check("array of structs b", sa[1].b, want1);

    v2d v = { i - 3, i + 1 };
    check("vector init lane0", v[0], want);
    check("vector init lane1", v[1], want1);

    v4f vf = { i - 3, 0, 0, i + 1 };
    check("float vector lane0", vf[0], (float)want);
    check("float vector lane3", vf[3], (float)want1);

    v2l vl = { i - 3, 0 };
    checkl("i64 vector lane0", vl[0], wantl);

    struct V sv = { 1, { i - 3, i + 1 } };
    check("struct vector member", sv.v[0], want);
    check("struct vector member 1", sv.v[1], want1);

    v2d cl = (v2d){ i - 3, i + 1 };
    check("vector compound literal", cl[0], want);

    v2d z = { 0.5, 0.25 };
    v2d sp = z + (i - 3);
    check("vector + scalar splat", sp[0], 0.5 + want);
    v2d sp2 = (i - 3) * z;
    check("scalar * vector splat", sp2[1], 0.25 * want);
    v2d sp3 = z;
    sp3 += i - 3;
    check("vector += scalar", sp3[0], 0.5 + want);

    v2l il = { 10, 20 };
    v2l isp = il + (i - 3);
    checkl("i64 vector splat", isp[0], 10 + wantl);

    check("creal", __builtin_creal(i - 3), want);

    double cl1 = (double){ i - 3 };
    check("scalar compound literal", cl1, want);

    check("cast", (double)(i - 3), want);
    check("return double", ret_d(i), want);
    checkl("return i64", ret_ll(i), wantl);
    check("return float", ret_f(i), (float)(i * 2 + 1));

    check("unary minus", -(i - 3), -want);
    checkl("unary minus i64", (long long)-(i - 3), -wantl);
    checkl("bitwise not i64", (long long)~(i - 3), ~wantl);
    double _Complex c = ret_c(i);
    check("complex return real", __real__ c, want);
    check("complex return imag", __imag__ c, 0.0);
    _Float128 fq = 0.5;
    fq += i - 3;
    check("_Float128 += int", (double)fq, 0.5 + want);
    v4qi qv = (v4qi)(i - 3);
    int back;
    __builtin_memcpy(&back, &qv, sizeof back);
    checkl("scalar->vector cast", back, i - 3);

    int n = i < 0 ? -i : i;
    double vla[n + 2];
    vla[n + 1] = i - 3;
    check("vla", vla[n + 1], want);
    checkl("vla size", (long long)(sizeof vla / sizeof vla[0]), n + 2);
}

int main(void) {
    for (int i = -7; i <= 7; i++)
        run(i);
    printf("init_conversion_value_width: %s (%d failures)\n", fails ? "FAIL" : "OK", fails);
    return fails != 0;
}
