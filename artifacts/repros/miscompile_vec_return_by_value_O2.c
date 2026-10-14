/* VECRET-1 reproducer (found 2026-10-13 during the AUD-ICE-1 spot-checks).
 *
 * A 16-byte vector returned BY VALUE from a function that computes it
 * (rather than passing a parameter through) reads the result's alloca
 * POINTER as the value: the printed value's high halves are a stack
 * address (0x7fff...), i.e. the return path copied the pointer, not the
 * pointee.  Parameters returned identity-style (idret) are correct, and
 * any assignment/global path is correct -- only the by-value return of a
 * computed vector rvalue is wrong.
 *
 *   lccc -O2 miscompile_vec_return_by_value_O2.c && ./a.out
 *     passret: <stackaddr-low> <0x7fff-high> 0 0     (wrong; gcc: 4 3 2 1)
 *   gcc  -O2 miscompile_vec_return_by_value_O2.c && ./a.out
 *     passret: 4 3 2 1
 */
#include <stdio.h>
typedef int v4 __attribute__((vector_size(16)));

v4 idret(v4 a) { return a; }                       /* correct through any path */
v4 passret(v4 a, v4 b) { return __builtin_shuffle(a, b); }

int main(void) {
    v4 x = {1, 2, 3, 4}, m = {3, 2, 1, 0};
    v4 r1 = idret(x);
    printf("idret:   %d %d %d %d\n", r1[0], r1[1], r1[2], r1[3]);
    v4 r2 = passret(x, m);
    printf("passret: %d %d %d %d\n", r2[0], r2[1], r2[2], r2[3]);
    return (r2[0] == 4 && r2[3] == 1) ? 0 : 1;
}
