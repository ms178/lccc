/* `__has_warning` must consult the compiler's ACTUAL warning registry
 * (EDG-transplant review finding: no blanket truth values). A flag lccc
 * implements (-Wreturn-type) yields 1; a flag no compiler knows
 * (-Wtotally-bogus-flag-xyz) yields 0 and sends ported sources down
 * their compatibility fallback.
 *
 * This is an LCCC self-check (see the .env sidecar), not a GCC differential.
 * Provenance: EDG Changes excerpt workflow (docs/edg_changes_c_extract.md). */
#include <stdio.h>

int main(void) {
  int known = 0, bogus = 0, groups = 0, pedantic = 0;
#if __has_warning("-Wreturn-type")
  known = 1;
#endif
#if __has_warning("-Wtotally-bogus-flag-xyz")
  bogus = 1;
#endif
#if __has_warning("-Wimplicit") && __has_warning("-Wno-unused")
  groups = 1;
#endif
#if __has_warning("-Wpedantic")
  pedantic = 1;
#endif
  printf("%d %d %d %d\n", known, bogus, groups, pedantic);
  return !(known && !bogus && groups && !pedantic);
}
