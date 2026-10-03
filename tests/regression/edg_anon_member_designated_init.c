/* Designated initializer reaching through a NONSTANDARD anonymous
 * union/struct nesting (EDG Changes 2019-09-12, EDGcpfe/21755,21756):
 * the classic EDG bug where this shape hung the front end in an infinite
 * loop. Guards lccc's initializer walker against the same pathology.
 *
 *   struct X { union { struct { int i; }; }; } x = {{{ .i = 1 }}};
 *
 * GCC accepts and initializes it — differential oracle. Provenance:
 * docs/edg_changes_c_extract.md (Apache-2.0 WITH LLVM-exception excerpts
 * from edgcpp/compiler src/Changes). */
#include <stdio.h>

struct X {
  union {
    struct {
      int i;
    };
  };
} x = {{{.i = 7}}};

int main(void) {
  printf("%d\n", x.i);
  return 0;
}
