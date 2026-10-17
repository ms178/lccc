/* sizeof / _Alignof of value expressions: C11 6.3.2.1p2-p4 (lvalue, array
 * and function conversions) apply to the operands of the comma and
 * conditional operators, so `sizeof((0, arr))` is the size of a pointer.
 * sizeof's "not converted" exception (6.5.3.4p4) covers only the operand as
 * written.  GNU extensions: sizeof(void) == sizeof of a void-typed value ==
 * sizeof(function designator) == 1.  Zero-length arrays keep size 0.
 * Every value is compared against GCC by the regression runner; the
 * constant-expression rows also pin that sema and lowering agree (an array
 * bound or _Static_assert that disagreed would fail to compile). */
#include <stdio.h>
#include <stddef.h>

int arr[4];
char str17[17];
struct holder { char c[17]; double d; } hold;
struct zla { int n; char tail[0]; } zl;
char (*pz)[0];
void vfn(void) {}
int ifn(int x) { return x; }
void *vp;

/* Constant-expression contexts: sema's evaluator must agree with lowering. */
_Static_assert(sizeof((0, arr)) == sizeof(int *), "comma decays");
_Static_assert(sizeof(1 ? arr : arr) == sizeof(int *), "conditional decays");
_Static_assert(sizeof(arr) == 4 * sizeof(int), "operand itself not converted");
_Static_assert(sizeof((0, ifn)) == sizeof(int (*)(int)), "function decays");
_Static_assert(sizeof((0, "abc")) == sizeof(char *), "string literal decays");
_Static_assert(sizeof("abc") == 4, "string literal operand not converted");
_Static_assert(sizeof((0, (void)0)) == 1, "void value (GNU)");
_Static_assert(sizeof(1 ? (void)0 : (void)0) == 1, "void conditional (GNU)");
int autoconf_shape[sizeof((0, arr)) == sizeof(int *) ? 1 : -1];
size_t g_comma = sizeof((0, arr));
size_t g_cond = sizeof(0 ? arr : (int *)0);
size_t g_void = sizeof((0, (void)0));
size_t g_zla = sizeof(zl.tail);

static struct holder mk(void) { return hold; }

#define P(e) printf("%-34s %zu\n", #e, (size_t)(e))

int main(void)
{
    int local[3];
    __typeof__((0, arr)) t_comma = 0;   /* int * */
    __typeof__(1 ? arr : arr) t_cond = 0; /* int * */

    P(sizeof(arr));
    P(sizeof((arr)));
    P(sizeof((0, arr)));
    P(sizeof(1 ? arr : arr));
    P(sizeof(0 ? arr : (int *)0));
    P(sizeof(1 ? (int *)0 : arr));
    P(sizeof((0, local)));
    P(sizeof(1 ? local : arr));
    P(sizeof((0, str17)));
    P(sizeof((0, hold.c)));
    P(sizeof((0, mk().c)));
    P(sizeof(hold.c));
    P(sizeof((0, "abc")));
    P(sizeof(1 ? "abc" : "de"));
    P(sizeof("abc"));
    P(sizeof((0, ifn)));
    P(sizeof(1 ? ifn : ifn));
    P(sizeof(ifn));
    P(sizeof(&arr));
    P(sizeof(*&arr));
    P(sizeof(arr + 0));
    P(sizeof((0, 0, arr)));
    P(sizeof(((void)0, arr)));
    P(sizeof(void));
    P(sizeof((void)0));
    P(sizeof(*vp));
    P(sizeof(vfn()));
    P(sizeof((0, (void)0)));
    P(sizeof((0, vfn())));
    P(sizeof(1 ? (void)0 : (void)0));
    P(sizeof(1 ? vfn() : vfn()));
    P(sizeof(zl.tail));
    P(sizeof(zl));
    P(sizeof(*pz));
    P(sizeof((0, *pz)));
    P(sizeof((0, zl.tail)));
    P(sizeof(t_comma));
    P(sizeof(t_cond));
    P(_Generic((0, arr), int *: 1, default: 0));
    P(_Generic(1 ? arr : arr, int *: 1, default: 0));
    P(_Alignof(void));
    P(__alignof__((0, arr)));
    P(__alignof__(arr));
    P(__alignof__((0, str17)));
    P(__alignof__((0, (void)0)));
    P(sizeof(autoconf_shape));
    P(g_comma);
    P(g_cond);
    P(g_void);
    P(g_zla);
    (void)t_comma; (void)t_cond;
    return 0;
}
