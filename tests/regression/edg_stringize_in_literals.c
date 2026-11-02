/* '#' inside string/char literals of a macro replacement list is text,
 * not the stringize operator (EDG-transplant review finding: the
 * stringize validator must be literal-aware). All three definitions are
 * valid C; a literal-unaware validator rejects them with "'#' is not
 * followed by a macro parameter".
 *
 *   #define F(x) "#"        #define G(x) '#'
 *   #define H(y) "use #x now"
 *
 * The truly invalid `#define BAD(y) #z` is covered by the preprocessor
 * corpus (stringize_skipped.c) and still diagnosed. GCC matches byte for
 * byte — differential oracle. Provenance: EDG Changes excerpt workflow
 * (docs/edg_changes_c_extract.md). */
#include <stdio.h>

#define F(x) "#"
#define G(x) '#'
#define H(y) "use #x now"

int main(void) {
  printf("%s %c %s\n", F(1), G(1), H(1));
  return 0;
}
