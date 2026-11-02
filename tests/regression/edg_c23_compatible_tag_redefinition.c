/* C23 compatible redefinition of tagged types (WG14 N3037; EDG Changes
 * 2026-09-15, EDGcpfe/25951,EDGcpfe/29043): a structure may be defined
 * more than once, and definitions in different scopes of one TU are
 * compatible when members correspond — so a block-scoped redefinition of
 * `struct S` can be passed to a function declared with the outer one.
 *
 * Requires -std=c2x: this is a dialect feature, and GCC 14 implements it
 * there too, so the differential oracle holds under that flag.
 * Provenance: docs/edg_changes_c_extract.md (Apache-2.0 WITH
 * LLVM-exception excerpts from edgcpp/compiler src/Changes). */
#include <stdio.h>

struct S {
  int i;
};

void f(struct S p) { printf("%d\n", p.i); }

void g(void) {
  struct S {
    int i;
  } s = {42};
  f(s); /* OK in C23: the two "struct S" types are compatible. */
}

int main(void) {
  g();
  return 0;
}
