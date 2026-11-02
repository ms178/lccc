/* _Float128 in the usual arithmetic conversions (C23 6.3.1.8): it is the
 * common type of any mix with a real arithmetic type, so the other operand
 * converts to binary128 (float/double/long double/integers/enums/__int128),
 * compound assignments compute in _Float128 and convert back, ++/-- step by
 * 1.0, conversions to _Bool compare with zero, and the f128 builtins have
 * type _Float128. A prototyped argument travels as its parameter's type:
 * _Float128 into double/int/long double/_Bool parameters, and double/int/
 * float/long double into _Float128 parameters (also after an int, where the
 * i686 slot needs its 16-byte alignment). Compared with GCC; every value is
 * exact in double. */
#include <stdio.h>

typedef _Float128 Q;
enum E { E3 = 3, EM = -2 };

static volatile int vi = 3;
static volatile unsigned vu = 7u;
static volatile float vf = 0.25f;
static volatile double vd = 1.5;
static volatile long long vll = -9;
static volatile unsigned long long vull = 11;
static volatile short vs = -2;
static volatile char vc = 5;
static volatile int zero = 0;

#define P(x) printf("%s=%.9g\n", #x, (double)(x))
#define PI(x) printf("%s=%lld\n", #x, (long long)(x))

static Q qid(Q x) { return x; }
static double as_d(double x) { return x; }
static int as_i(int x) { return x; }
static long double as_ld(long double x) { return x; }
static int as_b(_Bool x) { return x; }
static double from_q(int a, Q x, double b) { return (double)x + a + b; }
static int dec_as_i(int x) { return x; }

int main(void) {
    volatile Q q = 2.75Q, h = 0.5Q, n = -3.25Q, r;
    volatile Q nan = __builtin_nanf128("");

    /* Binary operators, both operand orders. */
    P(q + vf); P(vf + q); P(q - vd); P(vd - q); P(q * vi); P(vi * q); P(q / vu); P(vu / q);
    P(q + vll); P(vll - q); P(q * vull); P(q + vs); P(q - vc); P(q + 1); P(1 - q);
    P(q * 2.0f); P(q / 2.0);

    /* Compound assignment, _Float128 on either side. */
    r = q; r += vi; P(r); r = q; r -= vf; P(r); r = q; r *= vd; P(r); r = q; r /= vll; P(r);
    r = q; r += 1; P(r); r = q; r -= 0.5f; P(r); r = q; r *= vull; P(r);
    int i = 1; i += q; PI(i); double d = 0.5; d *= q; P(d); float f = 1.0f; f -= q; P(f);
    long long ll = 4; ll *= q; PI(ll); unsigned u32 = 9; u32 /= q; PI(u32);
    i = 5; i -= h; PI(i); i *= n; PI(i); unsigned ui = 10; ui /= h; PI(ui);
    d = 2; d /= q; P(d); f = 3; f += n; P(f);
    long double ld = 1.25L; ld *= q; P(ld);
    Q z = ld; z += ld; P(z); P(ld + q); P(q - ld); P(-q + -n);

    /* ++/--: pre/post, used and discarded. */
    r = q; r++; P(r); r = q; ++r; P(r); r = q; r--; P(r);
    Q x = q; Q y = x++; P(x); P(y); y = --x; P(x); P(y); x--; P(x); ++x; P(x);

    /* Comparisons, logical operators, conversions. */
    P(q > vi); P(vi < q); P(q == 2.75); P(q != vf); P(q >= vd); P(-q); P(!q);
    P(q > 2.5); P(2.5 > q); P(q == 2.75f); P(!h); P(h && vi); P(zero || q);
    P((int)q); P((unsigned)q); P((long long)-q); P((float)q); P((double)q); P((Q)vi + (Q)vf);
    P(qid(q) + vi);

    /* Conditional operator: constant and runtime conditions, either arm. */
    P(vi ? q : vd); P(0 ? q : vd); P(1 ? q : vd); P(zero ? q : vd); P(vi ? vd : q);
    P(0 ? vi : q); P(1 ? 7 : q);

    /* Enums convert as their compatible type. */
    enum E e = E3; P(q + e); P(e * q); P(q - EM); P(e < q); e = (enum E)q; PI(e);

    /* Conversion to _Bool and to narrow integers. */
    _Bool b = h; PI(b); b = nan; PI(b); b = 0.0Q; PI(b); b = q - q; PI(b);
    b = 1; b += h; PI(b); b = 0; b -= h; PI(b);
    signed char c = n; PI(c); short sh = q * 100; PI(sh); unsigned char uc = 200.75Q; PI(uc);

    /* Builtins have type _Float128. */
    P(__builtin_fabsf128(n)); P(__builtin_copysignf128(q, n)); P(__builtin_inff128() > q);
    P(__builtin_huge_valf128() == __builtin_inff128()); PI(nan != nan); PI(nan == nan);

    /* Arguments convert to their parameter types, in both directions. */
    P(as_d(q)); PI(as_i(n)); P(as_ld(q)); PI(as_b(q)); PI(as_b(-0.0Q)); PI(as_b(h));
    P(from_q(1, vd, 2.0)); P(from_q(1, vi, 2.0)); P(from_q(1, vf, 0.5)); P(from_q(-1, q, 0.25));
    P(qid(vd)); P(qid(vi)); P(qid(1.25L)); P(as_d(qid(vll)));
    volatile _Decimal128 dl = 7.0DL; PI(dec_as_i(dl));

    /* An lvalue with side effects is evaluated once. */
    Q arr[3] = {1.0Q, 2.0Q, 3.0Q};
    Q *p = arr;
    *p++ += 0.5Q; *p++ *= q;
    P(arr[0]); P(arr[1]); P(arr[2]); PI(p - arr);

#ifdef __SIZEOF_INT128__
    /* __int128 operands: unsigned __int128 shares the 128-bit carrier, so
     * its conversion must be a value conversion, not a bit copy. */
    __int128 w = 1000;
    unsigned __int128 u = ((unsigned __int128)1 << 100) + 12345;
    __int128 s = -((__int128)1 << 90);
    P(q + w); P((__int128)(q * 4));
    P((Q)u); P((Q)s); P((Q)(unsigned __int128)5); P((Q)(__int128)-7);
    Q from_u = u; P(from_u); P(u > q);
    PI((unsigned __int128)(q * 1e30Q) >> 64); PI((__int128)n);
#endif
    return 0;
}
