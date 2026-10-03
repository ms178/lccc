/* `__has_warning` must consult the compiler's ACTUAL warning registry
 * (EDG-transplant review finding: no blanket truth values). A flag lccc
 * implements (-Wreturn-type) yields 1; a flag no compiler knows
 * (-Wtotally-bogus-flag-xyz) yields 0 and sends ported sources down
 * their compatibility fallback.
 *
 * GCC 14 supports __has_warning and agrees on both verdicts —
 * differential oracle. Provenance: EDG Changes excerpt workflow
 * (docs/edg_changes_c_extract.md). */
#include <stdio.h>

int main(void) {
  int known = 0, bogus = 0;
#if __has_warning("-Wreturn-type")
  known = 1;
#endif
#if __has_warning("-Wtotally-bogus-flag-xyz")
  bogus = 1;
#endif
  printf("%d %d\n", known, bogus);
  return 0;
}
