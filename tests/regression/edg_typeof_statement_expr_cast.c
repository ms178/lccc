/* typeof through a GNU statement expression used as an initializer
 * (shape reduced from EDG Changes 2023-02-07, EDGcpfe/25874 — the
 * __builtin_has_attribute(&({ ... })) internal-error repro, minus the
 * attribute probe which is out of scope for the runnable corpus):
 *
 *   ({ typeof(*sptr) *var = 0; ((typeof(*(sptr)) *)(var)); })
 *
 * Exercises statement expressions, typeof of a pointer-to-incomplete-ish
 * struct member, and casts between typeof-derived pointers. GCC matches —
 * differential oracle. Provenance: docs/edg_changes_c_extract.md
 * (Apache-2.0 WITH LLVM-exception excerpts from edgcpp/compiler
 * src/Changes). */
#include <stdio.h>

struct S {
  char *str;
} * sptr;

int main(void) {
  void *r = ({
    typeof(*sptr) *var = 0;
    ((typeof(*(sptr)) *)(var));
  });
  printf("ok %d\n", r == 0);
  return 0;
}
