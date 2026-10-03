/* C23 empty initializers `{}` (WG14 N2998; EDG Changes 2026-09-14,
 * EDGcpfe/25946,EDGcpfe/29042 — C-score +15, the highest-scored EDG
 * Changes entry in docs/edg_changes_c_extract.md).
 *
 * An empty initializer may initialize any complete type; every member or
 * element takes a zero-valued constant. GNU C accepts this as an extension
 * in all recent modes, so GCC is a valid differential oracle.
 *
 * Provenance: docs/edg_changes_c_extract.md (Apache-2.0 WITH
 * LLVM-exception excerpts from edgcpp/compiler src/Changes). */
#include <stdio.h>

int main(void) {
  int i = {};
  double d = {};
  void *p = {};
  unsigned char buf[8] = {};
  for (int k = 0; k < 8; k++) {
    if (buf[k] != 0) {
      printf("BAD_ELEMENT %d\n", k);
      return 1;
    }
    buf[k] = (unsigned char)(buf[k] + 1);
  }
  printf("%d %d %d %u\n", i, (int)d, p == 0, buf[7]);
  return 0;
}
