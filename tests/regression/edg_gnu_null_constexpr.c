/* GNU-mode extended constant expressions (EDG Changes 2019-10-08,
 * EDGcpfe/21770 + 2023-03-27, EDGcpfe/26183):
 *   - casts of null pointers are constant expressions in GNU C mode:
 *     _Static_assert((int*)(void*)0 == (int*)0, ...)
 *   - !(void*)0 is accepted where an integral constant-expression is
 *     expected;
 *   - comparing a variable with itself is folded to a constant, so
 *     `struct S { int i: 1+(x == x); }` with non-constant x is legal.
 *
 * GCC accepts all three in its default GNU mode — differential oracle.
 * Provenance: docs/edg_changes_c_extract.md (Apache-2.0 WITH
 * LLVM-exception excerpts from edgcpp/compiler src/Changes). */
#include <stdio.h>

_Static_assert((int *)(void *)0 == (int *)0, "null cast const");
_Static_assert(!(void *)0, "null logical const");

int x;
struct S {
  int i : 1 + (x == x);
};

int main(void) {
  printf("ok %d\n", (int)sizeof(struct S));
  return 0;
}
