/* ISO DR423: type qualifiers on function return types are ignored, so a
 * redeclaration adding a top-level qualifier on the return type is
 * compatible (EDG Changes 2023-02-07, EDGcpfe/25890):
 *
 *   int f(void);
 *   int const f(void);   // error in strict C11, OK per DR423
 *
 * GCC accepts this pair in its default mode — differential oracle.
 * Provenance: docs/edg_changes_c_extract.md (Apache-2.0 WITH
 * LLVM-exception excerpts from edgcpp/compiler src/Changes). */
#include <stdio.h>

int f(void) { return 3; }
int const f(void);

int main(void) {
  printf("%d\n", f());
  return 0;
}
