#include <stdio.h>
#include <stddef.h>
struct Outer { int a; union { int b; float f; }; struct { short c, d; }; };
struct Aligned { _Alignas(16) char c; int i; };
_Static_assert(sizeof(int) == 4, "int must be 4 bytes");
_Static_assert(offsetof(struct Outer, d) == 12, "layout");
static int helper(void) { return 3; }
int main(void) {
    struct Outer o = { 1, { 2 }, { 3, 4 } };
    printf("%d %d %d %d\n", o.a, o.b, o.c, o.d);
    printf("%zu %zu\n", sizeof(struct Outer), offsetof(struct Outer, d));
    printf("%zu %zu\n", sizeof(struct Aligned), _Alignof(struct Aligned));
    printf("%d\n", helper());
    int n = 4; int vla[n]; vla[0] = 1; vla[3] = 4;
    printf("%zu %d %d\n", sizeof vla, vla[0], vla[3]);
    _Static_assert(1, "in block");
    printf("%d\n", __builtin_constant_p(3 + 4) ? 1 : 0);
    printf("%d\n", __builtin_types_compatible_p(int, signed) ? 1 : 0);
    printf("%d\n", __builtin_types_compatible_p(int, unsigned) ? 1 : 0);
    printf("%d\n", __builtin_offsetof(struct Outer, d));
    return 0;
}
