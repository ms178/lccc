/*
 * Enum and enumerator types, as GCC 16.2 and Clang 23.1.3 both define them
 * (measured identically under -std=gnu17, gnu23 and c11 on x86-64, i686 and
 * AArch64):
 *  - an enum's compatible type is `unsigned int` without negative
 *    enumerators and `int` with one; 4 bytes only when ONE 32-bit type holds
 *    every value, else `long`/`unsigned long` (LP64) or `long long`/
 *    `unsigned long long` (ILP32);
 *  - after completion, every enumerator is `int` when all of them fit in
 *    `int`, otherwise every enumerator has the enum's compatible type;
 *  - an enum bit-field keeps the enum's signedness;
 *  - an enum type behaves as its compatible type in `_Generic`, promotions,
 *    `sizeof`, `_Alignof`/`__alignof__` (i386: 4/8 for an 8-byte enum) and
 *    VLA sizing. Each fact is asserted statically AND checked at run time,
 *    because sema and lowering once answered these with different rules.
 */
#include <stdio.h>

#define TY(x) _Generic((x), int: 1, unsigned: 2, long: 3, unsigned long: 4, \
                       long long: 5, unsigned long long: 6, signed char: 7, \
                       unsigned char: 8, default: 0)
#if __SIZEOF_LONG__ == 8
#define S64 3
#define U64 4
#else
#define S64 5
#define U64 6
#endif

enum Small { SA, SB };
enum Neg { NA = -1, NB };
enum Big { BG = 0x80000000u };
enum Huge { HG = 0x100000000LL, HG2 };
enum Mix { MA = 0, MB = 0x100000000LL };
enum MixU { UA = 1, UB = 0x80000000u };
enum MixN { XA = -1, XB = 0x80000000u };

_Static_assert(TY(SA) == 1 && TY(NA) == 1, "int enumerators");
_Static_assert(TY(BG) == 2 && TY(UA) == 2 && TY(UB) == 2, "unsigned int enum constants");
_Static_assert(TY(HG) == U64 && TY(MA) == U64 && TY(MB) == U64, "unsigned 64-bit enum constants");
_Static_assert(TY(XA) == S64 && TY(XB) == S64, "signed 64-bit enum constants");
_Static_assert(sizeof(MA) == 8 && sizeof(XA) == 8 && sizeof(SA) == 4, "enumerator sizes");
_Static_assert(sizeof(enum MixN) == 8 && sizeof(enum MixU) == 4, "enum sizes");

enum __attribute__((packed)) PU { PA, PB = 200 };
enum __attribute__((packed)) PS { PN = -3 };
static enum Small vs;
static enum Mix vm;
static enum PU vpu;
static enum PS vps;
_Static_assert(TY(vs) == 2 && TY(vm) == U64 && TY(vpu) == 8 && TY(vps) == 7, "enum objects");
_Static_assert(TY((enum Neg)0) == 1 && TY((enum MixN)0) == S64, "enum casts");
_Static_assert(TY(vs + 0) == 2 && TY(vm + 0) == U64 && TY(vpu + 0) == 1 && TY(+vps) == 1,
               "enum promotions");
_Static_assert(sizeof(enum Mix) == 8 && sizeof((enum Mix)0) == 8 && sizeof vm == 8 &&
                   sizeof(enum PU) == 1,
               "enum type sizes");
_Static_assert(_Alignof(enum Mix) == _Alignof(unsigned long long) &&
                   __alignof__(enum Mix) == __alignof__(unsigned long long) &&
                   _Alignof(enum PU) == 1,
               "enum type alignment");

struct BF {
    enum Neg n : 2;
    enum Small s : 2;
    enum { AN = -2, AP } anon : 2;
};
typedef enum Neg TNeg;
struct TBF { TNeg t : 3; };

static int enum_type_runtime(int n)
{
    enum PU pv[n];
    enum Mix mv[n];
    int fails = 0;
    fails += TY(vs) != 2 || TY(vm) != U64 || TY(vpu) != 8 || TY(vps) != 7;
    fails += TY(vs + 0) != 2 || TY(vpu + 0) != 1 || TY((enum MixN)0) != S64;
    fails += sizeof pv != (size_t)n || sizeof mv != 8 * (size_t)n;
    fails += sizeof(enum Mix) != 8 || _Alignof(enum Mix) != _Alignof(unsigned long long);
    fails += __alignof__(enum Mix) != __alignof__(unsigned long long) || _Alignof(enum PU) != 1;
    return fails;
}

int main(int argc, char **argv)
{
    (void)argv;
    enum Small s = SA;
    enum Neg n = NB;
    enum Huge h = HG;
    enum MixN x = XA;
    int fails = 0;
    fails += TY(s) != 2 || TY(n) != 1 || TY(h) != U64 || TY(x) != S64;
    fails += (s - 1) < 0;                       /* unsigned int arithmetic */
    fails += !((n - 1) < 0);                    /* int arithmetic */
    fails += (unsigned long long)HG2 != 0x100000001ULL;
    struct BF bf = { NA, SB, AN };
    struct TBF tbf = { -3 };
    fails += bf.n != -1 || !(bf.n < 0) || bf.s != 1 || bf.anon != -2 || tbf.t != -3;
    {
        enum Small { SV = -5 };                 /* shadows the outer tag */
        enum Small inner = SV;
        fails += !((inner - 1) < 0);
    }
    enum Small after = SA;                      /* outer tag restored */
    fails += (after - 1) < 0;
    fails += enum_type_runtime(argc + 2);
    puts(fails ? "enum type semantics: FAIL" : "enum type semantics: ok");
    return fails != 0;
}
