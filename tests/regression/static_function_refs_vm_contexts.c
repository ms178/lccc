/*
 * A static function referenced only from a VLA bound, a variably modified
 * parameter type, __builtin_convertvector, or typeof of a VLA type is still
 * called, so it must be emitted; one referenced only from an unevaluated
 * operand (sizeof, typeof, _Generic's other associations) is not, and its
 * callees need not exist.  Before the fix the dead-static pre-filter missed
 * the first group (undefined symbols at link time), and -O0/-O1 never ran
 * the IR reachability pass that removes the second.  Compiled at -O0.
 */
#include <stdio.h>

typedef float v4f __attribute__((vector_size(16)));
typedef int v4i __attribute__((vector_size(16)));

static v4i vec(void) { v4i v = {1, 2, 3, 4}; return v; }
static int n1(void) { return 3; }
static int n2(void) { return 5; }
static int n4(void) { return 4; }
static int n5(void) { return 6; }
static int n6(void) { return 7; }
static int n7(void) { return 3; }
static int vm_param(int (*q)[n7()]) { return (int)sizeof *q; }

extern int never_defined(void);
static inline int only_sizeof(void) { return never_defined(); }
static inline int only_typeof(void) { return never_defined(); }
static inline int only_generic(void) { return never_defined(); }

int main(void)
{
    v4f fv = __builtin_convertvector(vec(), v4f);
    int a[n1()];
    int m = 1;
    typeof(int[n4() + m]) c;
    typedef int T[n5()];
    T t;
    int k = 2;
    int b[3];
    typeof(only_typeof()) z = 0;
    printf("%g %zu %zu %zu %zu %zu %d %d\n", fv[3], sizeof a, sizeof(int[n2()]), sizeof c,
           sizeof t, sizeof(char[k][n6()]), vm_param(&b),
           (int)sizeof(only_sizeof()) + z + _Generic(0, long: only_generic(), default: 0));
    return 0;
}
