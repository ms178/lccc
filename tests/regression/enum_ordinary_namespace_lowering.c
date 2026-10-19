/*
 * Enumerators share C's ordinary-identifier namespace with objects,
 * functions and typedef names (C11 6.2.1p4, 6.2.3): the innermost
 * declaration decides, in both directions, and an enum defined in an
 * expression's type-name is in scope from that point on (6.2.1p7).
 *
 * Before the fix lowering resolved identifiers by name with inconsistent
 * precedence: an enumerator beat a shadowing local, parameter or static
 * local when typing (`double A` read as int: f/g/h/s printed `0 1 0 0`),
 * a local beat an inner-block enumerator, and enumerators or tags defined
 * in sizeof/cast/_Alignof/__builtin_types_compatible_p/va_arg type-names
 * were unknown (link errors, or an `enum T` with a negative enumerator
 * typed unsigned).  Expected output is GCC 16.2's.
 */
#include <stdarg.h>
#include <stdio.h>

enum { A = 1, B = 2, C = 3 };

/* An object hides a file-scope enumerator of the same name. */
static double f(void) { double A = 2.5; return A * 2; }
static long long g(void) { long long B = 1LL << 40; return B + 1; }
static double h(double A) { return A / 2; }
static double s(void) { static double C = 1.25; return C * 4; }
static int z(void) { double A = 0; (void)A; return (int)sizeof A + (int)sizeof B; }

/* An inner enumerator hides an outer object. */
static int r;
static int w(void) { double A = 8.5; (void)A; { enum { A = 3 }; return (int)sizeof A + A; } }
static int f1(void) { int A = 5; { enum { A = 3 }; _Static_assert(A == 3, ""); r += A; } return A; }
static int f7(void) { enum { F = 5 }; { enum { F = F + 1 }; return F; } }
static int f8(void) { { int A; (void)A; } return __builtin_choose_expr(A, 40, 0); }
static int t3(void) { int A = 7; (void)A; { enum { A = 3 }; switch (7) { case A + 4: return 1; } } return 0; }

/* Enumerators declared in an expression's type-name. */
int g1 = (int)sizeof(enum G1 { GN = -1 }) + GN;
enum G1 gv = GN;
static int va(int n, ...)
{
    va_list ap;
    va_start(ap, n);
    int v = va_arg(ap, enum { V = 2 }) + V;
    va_end(ap);
    return v;
}
static int e1(void) { return (int)sizeof(enum { E = 30 }) + E; }
static int e2(void) { int b = (enum { R = 6 })0 + R; return b; }
static int e3(void) { return ((struct { enum { S = 7 } m; }){ S }).m + S; }
static int e4(void) { return __builtin_types_compatible_p(enum { T = 1 }, int) * 0 + T; }
static int e5(void) { return (int)_Alignof(enum { U = 9 }) * 0 + U; }
static int e6(void) { int s = 0; for (enum { K = 4 } e = K; e < 6; e++) s += e + K; return s; }

/* A tag defined in a folded type-name keeps its definition (signedness). */
static int t1(void) { int n = (int)sizeof(enum T1 { NEG = -1, P = 1 }); enum T1 x = NEG; return (x < 0) + n; }
static int t4(void) { int x; x = 1 + (int)sizeof(enum T4 { M = -2 }); enum T4 y = M; return (y < 0) + x; }
static int t5(void) { { (void)sizeof(enum T5 { Z = -3 }); } enum T5 { Z5 = 1 } q = Z5; return q; }
static int t6(void) { int a[sizeof(enum T6 { KK = -4 })]; enum T6 v = KK; return (v < 0) + (int)sizeof a; }

/* A selection or iteration statement and each of its substatements is a
   block (C11 6.8.4p3, 6.8.5p5): a tag or enumerator its type-names declare
   ends with it. */
enum TB { TB1 = 1 }; /* unsigned underlying type */
static int blocks(void)
{
    int x = 0, z = 0, w, q = 3;
    if (x == 0)
        x = sizeof(enum TB { TBN = -1 });
    enum TB y = TB1; /* the file-scope TB */
    while (z < 1)
        z += (int)sizeof(enum TB { TBM = -2 });
    enum TB v = TB1;
    if (x)
        w = (enum { q = 7 })0 + q; /* the enumerator q */
    else
        w = 0;
    return (x == 4) + 2 * (y > -1) + 4 * (z == 4) + 8 * (v > -1) + 16 * (w == 7) + 32 * (q == 3);
}

/* A local shadowing an enumerator keeps its own type as a call argument,
   also when the enumerator is declared later in the file (reduced from
   binutils 2.47 gas/config/tc-i386.c i386_generate_nops, whose
   `offsetT disp` was narrowed to int before fits_in_imm7 at -O0). */
enum earlier_kind { kb_unused0, kb };
static int wide(long long v) { return v > 0x7fffffffLL; }
static int arg_before(long long count)
{
    long long kb = count - 2;
    return wide(kb) * 10 + (int)sizeof(kb);
}
static int arg_after(long long count)
{
    long long ka = count - 2;
    return wide(ka) * 10 + (int)sizeof(ka);
}
enum later_kind { ka_unused, ka, kb_unused };

int main(void)
{
    printf("%g %lld %g %g %d\n", f(), g(), h(5.0), s(), z());
    int v1 = f1();
    printf("%d %d %d %d %d %d %d\n", w(), v1, r, f7(), f8(), t3(), va(1, 40));
    printf("%d %d %d %d %d %d %d %d %d\n", g1, gv < 0, e1(), e2(), e3(), e4(), e5(), e6(), t1());
    printf("%d %d %d %d %d %d\n", t4(), t5(), t6(), blocks(),
           arg_before(0x100000002LL), arg_after(0x100000002LL));
    return !(f() == 5.0 && g() == (1LL << 40) + 1 && h(5.0) == 2.5 && s() == 5.0 && z() == 12
             && w() == 7 && v1 == 5 && r == 3 && f7() == 6 && f8() == 40 && t3() == 1
             && va(1, 40) == 42 && g1 == 3 && gv < 0 && e1() == 34 && e2() == 6 && e3() == 14
             && e4() == 1 && e5() == 9 && e6() == 17 && t1() == 5 && t4() == 6 && t5() == 1
             && t6() == 17 && blocks() == 53
             && arg_before(0x100000002LL) == 18 && arg_after(0x100000002LL) == 18);
}
