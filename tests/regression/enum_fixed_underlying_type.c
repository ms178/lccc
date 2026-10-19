/* C23 fixed underlying enum types (6.7.2.2): layout, types, values. */
#include <stdio.h>
#include <stddef.h>
enum S : short { SA = -2, SB };
enum U : unsigned char { UA = 250, UB };
enum L : unsigned long long { LA = 0xffffffffffffffff };
enum F : int;                       /* forward: complete */
enum F : int { FA = 7 };
typedef signed char I8;
enum T : const I8 { TA = -128 };
enum B : _Bool { BF, BT };
struct P { char c; enum S s; enum U u; enum L l; };
_Static_assert(sizeof(enum S) == 2 && _Alignof(enum S) == 2);
_Static_assert(sizeof(enum U) == 1 && sizeof(enum L) == 8 && sizeof(enum T) == 1);
_Static_assert(_Generic(SA, enum S: 1, default: 0));
_Static_assert(_Generic(SA + 0, int: 1, default: 0));
_Static_assert(_Generic((enum S)0, short: 1, default: 0));
_Static_assert(_Generic(LA, unsigned long long: 1, default: 0));
_Static_assert(LA == 0xffffffffffffffff && UB == 251 && SB == -1);
_Static_assert(offsetof(struct P, u) == 4 && offsetof(struct P, l) == 8);
struct Bits { enum U x : 3; enum S : 2; enum S y : 4; };
static enum U ub = UB;
static enum F *fp;
int main(void) {
    enum U u = 255; u++;            /* wraps as unsigned char */
    enum S s = SA; s--;
    struct Bits b = { 5, -3 };
    struct P p = { 1, SB, UB, LA };
    enum F f = FA; fp = &f;
    printf("%d %d %d %llu %d %d %d %d %d %zu %d %d\n", u, s, ub, (unsigned long long)p.l,
           p.s, p.u, b.x, b.y, *fp, sizeof(struct P), TA, BT);
    return 0;
}
