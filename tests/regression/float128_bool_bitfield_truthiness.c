/* Conversions to _Bool and to bit-fields from floating operands, and the
 * truth value of _Float128 operands.
 * - A compound assignment computes in the common floating type and converts
 *   only the result (C11 6.5.16.2p3): `bf *= 0.5` halves the field; it must
 *   not truncate 0.5 to 0 first. Covered for binary, _Float128 and decimal
 *   right operands and for a _Bool bit-field.
 * - Conversion to _Bool is `value != 0` (C11 6.3.1.2): `b += 0.5` stores 1.
 * - A _Float128 condition tests its value: -0.0Q is false, any value whose
 *   low carrier bits are zero (2.75Q) is true.
 * - Every conversion to _Bool (initializer, braced scalar, array, struct and
 *   bit-field member, assignment, argument, return) compares the _Float128
 *   or decimal value with zero; the integer carrier does not encode zero
 *   uniquely (-0.0Q; BID zeros with any exponent, such as 0.00DD).
 * Compared with GCC. */
#include <stdio.h>

typedef _Float128 Q;
struct B {
    int a : 5;
    unsigned b : 7;
    _Bool f : 1;
    long long w : 40;
};

static volatile int zero = 0;

#define PI(x) printf("%s=%lld\n", #x, (long long)(x))

static _Bool to_bool(Q a) { return a; }
static _Bool dec_to_bool(_Decimal64 a) { return a; }
static int take_bool(_Bool b) { return b; }

int main(void) {
    volatile Q q = 2.75Q, h = 0.5Q, mz = -0.0Q;

    _Bool b = 0; b += 0.5; PI(b);
    b = 1; b -= 0.5; PI(b);
    b = 1; b *= 0.25; PI(b);
    b = 0; b += h; PI(b);

    struct B s = {3, 9, 0, 1000};
    s.a *= 0.5; PI(s.a); s.b -= 0.5; PI(s.b); s.f += 0.5; PI(s.f); s.w *= 2.5; PI(s.w);
    s.a = 3; s.a *= h; PI(s.a); s.b = 9; s.b /= h; PI(s.b);
    s.f = 0; s.f += h; PI(s.f); s.w -= q; PI(s.w);
    s.a = -7; s.a += 0.75f; PI(s.a); s.a = 5; s.a -= 9.5; PI(s.a);
    s.a = 3; s.a += 1.25DD; PI(s.a); s.b = 10; s.b *= 0.5DF; PI(s.b);

    PI(zero || q); PI(q || zero); PI(q && h); PI(!q); PI(!mz);
    PI(mz ? 1 : 2); PI(mz || zero); PI(mz && 1);
    int k = 0;
    if (q) k |= 1;
    if (mz) k |= 2;
    if (!mz) k |= 4;
    PI(k);
    int it = 0;
    volatile Q c = 3.0Q;
    while (c) { c -= 1; it++; }
    PI(it);
    for (Q x = 0.5Q; x; x = 0) it += 10;
    PI(it);
    PI(q ? 1 : 2); PI(h > 0 ? 7 : 8);
    PI(to_bool(q)); PI(to_bool(h)); PI(to_bool(mz)); PI(to_bool(0.0Q));
    PI(to_bool(__builtin_nanf128("")));

    /* Each conversion site, with zeros whose carrier is nonzero. */
    _Bool i1 = mz, i2 = {mz}, i3 = q;
    PI(i1); PI(i2); PI(i3);
    _Bool ba[3] = {mz, q, h};
    PI(ba[0]); PI(ba[1]); PI(ba[2]);
    struct B t = {0, 0, mz, 0};
    PI(t.f);
    struct B t2 = {.f = q};
    PI(t2.f);
    t.f = mz; PI(t.f); t.f = h; PI(t.f);
    i1 = mz; PI(i1); i1 = q; PI(i1);
    PI(take_bool(mz)); PI(take_bool(q)); PI(take_bool(h));

    volatile _Decimal64 dz = 0.00DD, dn = -0.0DD, dh = 0.5DD;
    volatile _Decimal32 fz = 0.000DF;
    volatile _Decimal128 lz = -0.0000DL;
    _Bool d1 = dz, d2 = dn, d3 = fz, d4 = lz, d5 = dh, d6 = {dz};
    PI(d1); PI(d2); PI(d3); PI(d4); PI(d5); PI(d6);
    _Bool da[3] = {dz, fz, dh};
    PI(da[0]); PI(da[1]); PI(da[2]);
    struct B t3 = {0, 0, dn, 0};
    PI(t3.f);
    t3.f = lz; PI(t3.f); t3.f = dh; PI(t3.f);
    d1 = fz; PI(d1); d1 = dh; PI(d1);
    PI(take_bool(dz)); PI(take_bool(lz)); PI(take_bool(dh));
    PI(dec_to_bool(dz)); PI(dec_to_bool(dn)); PI(dec_to_bool(dh));
    PI(!dz); PI(dz ? 1 : 2); PI(dn || zero);
    /* _Decimal128 truth tests compare with the canonical BID128 zero. */
    volatile _Decimal128 l0 = 0.0DL, l1 = 1.0DL;
    PI(!lz); PI(!l0); PI(!l1); PI(lz ? 1 : 2); PI(l1 ? 1 : 2); PI(l0 || zero); PI(l1 && 1);
    if (l0) it = -1;
    PI(it);
    return 0;
}
