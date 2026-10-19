/* Implicit conversions from and to _Float128 (and C23 decimal) in the
 * initializers of AUTOMATIC objects, through every local initializer path:
 * braced scalars, 1-D and 2-D arrays, struct members (scalar and array),
 * typedef'd _Bool arrays, vector lanes, and decimal <-> binary elements.
 * Each site is driven by constants (folded at compile time) and by volatile
 * sources (converted at run time).
 *
 * The _Float128 and decimal carriers are 16/8/4-byte integers in the IR, so
 * an element conversion chosen by IR type instead of C type reinterprets
 * bits.  Pinned regressions: `int a[] = {2.75Q}` stored the payload's low
 * bits, `_Float128 q[] = {1, 2.5}` stored the integers 1 and 2, float and
 * long long elements from _Float128 were garbage, `_Decimal64 d[] = {2.75Q}`
 * crashed the compiler (_Float128 was not a valid decimal source), 2-D and
 * struct-member arrays took the same IR-typed path, and the i686 -O0 build
 * crashed in the scalar cast emitter.  `static long double = 0.1DD` must
 * round once (was rounded through double: ...ccd000 instead of ...cccd).
 *
 * All values are exact or correctly rounded and printed as bit patterns, so
 * nothing depends on printf's rounding. */
#include <stdio.h>
#include <string.h>

typedef _Float128 Q;
typedef _Bool B3[3];
typedef float v4sf __attribute__((vector_size(16)));

struct S {
    int i;
    Q q;
    float f;
    _Bool b;
    long long l;
};
struct T {
    Q qa[2];
    int ia[2];
};

static void bits(const char *name, const void *p, size_t n)
{
    const unsigned char *b = p;
    printf("%s:", name);
    while (n--)
        printf("%02x", b[n]);
    printf("\n");
}

static long double s_ld = 0.1DD;
static _Float128 s_qd = 0.1DL;

int main(void)
{
    volatile Q vq = 2.75Q;
    volatile int vi = 3;
    volatile double vd = 0.1;
    volatile _Decimal64 vdd = 0.1DD;

    bits("s_ld", &s_ld, 10);
    bits("s_qd", &s_qd, 16);

    int bi = {2.75Q};
    Q bq = {7};
    float bf = {0.1Q};
    printf("braced %d\n", bi);
    bits("bq", &bq, 16);
    bits("bf", &bf, 4);

    Q q1[4] = {1, 2.5, vi, vd};
    for (int k = 0; k < 4; k++)
        bits("q1", &q1[k], 16);
    int i1[4] = {2.75Q, -3.5Q, vq, 1e3Q};
    printf("i1 %d %d %d %d\n", i1[0], i1[1], i1[2], i1[3]);
    float f1[2] = {0.1Q, vq};
    bits("f1", f1, 8);
    double d1[2] = {0.1Q, vq};
    bits("d1", d1, 16);
    long double l1[2] = {0.1Q, vq};
    bits("l1_0", &l1[0], 10);
    bits("l1_1", &l1[1], 10);
    long long ll1[3] = {2.75Q, -1.5Q, vq};
    printf("ll1 %lld %lld %lld\n", ll1[0], ll1[1], ll1[2]);
    unsigned long long u1[2] = {1.8e19Q, vq};
    printf("u1 %llu %llu\n", u1[0], u1[1]);
    Q fromu[2] = {18446744073709551615ULL, -1};
    bits("fu0", &fromu[0], 16);
    bits("fu1", &fromu[1], 16);

    Q q2[2][2] = {{1, 2.5}, {vi, vd}};
    bits("q2_00", &q2[0][0], 16);
    bits("q2_01", &q2[0][1], 16);
    bits("q2_10", &q2[1][0], 16);
    bits("q2_11", &q2[1][1], 16);
    int i2[2][2] = {{2.75Q, -3.5Q}, {vq, 1e3Q}};
    printf("i2 %d %d %d %d\n", i2[0][0], i2[0][1], i2[1][0], i2[1][1]);

    struct S s = {2.75Q, 3, 0.1Q, 0.5Q, -1.5Q};
    printf("S %d %d %lld\n", s.i, s.b, s.l);
    bits("S.q", &s.q, 16);
    bits("S.f", &s.f, 4);
    struct S s2 = {vq, vi, vq, vq, vq};
    printf("S2 %d %d %lld\n", s2.i, s2.b, s2.l);
    bits("S2.q", &s2.q, 16);
    bits("S2.f", &s2.f, 4);
    struct T t = {{1, vd}, {2.75Q, vq}};
    bits("T.q0", &t.qa[0], 16);
    bits("T.q1", &t.qa[1], 16);
    printf("T %d %d\n", t.ia[0], t.ia[1]);

    B3 b3 = {0.5Q, 0.0Q, -0.0Q};
    _Bool bv[2] = {vq, 0.0Q};
    printf("b3 %d %d %d bv %d %d\n", b3[0], b3[1], b3[2], bv[0], bv[1]);

    v4sf v = {2.5Q, 0.1Q, vq, 1};
    bits("v", &v, 16);

    _Decimal64 dd[3] = {2.75Q, 1, vq};
    bits("dd0", &dd[0], 8);
    bits("dd1", &dd[1], 8);
    bits("dd2", &dd[2], 8);
    _Decimal32 df = vq;
    bits("df", &df, 4);
    _Decimal128 dl = vq;
    bits("dl", &dl, 16);
    Q fromdd[3] = {0.1DD, 2, vdd};
    bits("fd0", &fromdd[0], 16);
    bits("fd1", &fromdd[1], 16);
    bits("fd2", &fromdd[2], 16);
    long double ldd[2] = {0.1DD, vdd};
    bits("ldd0", &ldd[0], 10);
    bits("ldd1", &ldd[1], 10);
    int idd[2] = {2.75DD, -1.5DD};
    printf("idd %d %d\n", idd[0], idd[1]);
    return 0;
}
