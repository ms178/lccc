// _Float128 (IEEE binary128) end-to-end: literal lexing, soft-float arithmetic
// via libgcc TF helpers, single-XMM SysV ABI across function boundaries, and
// conversions in both directions. Returns nonzero on the first failure.
#include <stddef.h>

static int fails = 0;
#define CHECK(cond) do { if (!(cond)) { fails++; } } while (0)

_Float128 add3(_Float128 a, _Float128 b, _Float128 c) { return a + b + c; }
_Float128 mix(_Float128 a, double b, _Float128 c, int d) { return a * c + (_Float128)b + (_Float128)d; }
_Float128 ident(_Float128 x) { return x; }

static _Float128 g = 42.0F128;

int main(void) {
    _Float128 a = 1.5F128, b = 2.25F128, c = 3.125F128;
    CHECK(sizeof(_Float128) == 16);
    CHECK(_Alignof(_Float128) == 16);
    CHECK(a + b == 3.75F128);
    CHECK(a * b == 3.375F128);
    CHECK(b / a == 1.5F128);
    CHECK(b - a == 0.75F128);
    CHECK(a < b && b <= c && c > a && a != b);
    CHECK(a >= a && a <= a && a == a);
    CHECK(-a == -1.5F128);
    CHECK(g == 42.0F128);
    double d = (double)a;
    CHECK(d == 1.5);
    float f = (float)b;
    CHECK(f == 2.25f);
    long long ll = (long long)c;
    CHECK(ll == 3);
    CHECK((_Float128)d == 1.5F128);
    CHECK((_Float128)7 == 7.0F128);
    CHECK((_Float128)9007199254740993LL == 9007199254740993.0F128);
    CHECK(ident(a) == 1.5F128);
    CHECK(add3(a, b, c) == 6.875F128);
    CHECK(mix(a, 0.5, c, 2) == (1.5F128 * 3.125F128 + 0.5F128 + 2.0F128));
    return fails != 0;
}
