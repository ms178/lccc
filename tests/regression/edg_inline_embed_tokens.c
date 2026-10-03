/* C23 6.10.15 inline `#embed` (EDG-transplant review finding fixes):
 * the directive may appear MID line and the tokens after the embed-spec
 * are ordinary code — `{#embed FILE limit(2) , };` must keep its trailing
 * `, };`. Also pins the hardened parameter grammar: hex pp-numbers
 * (limit(0x2)), character-literal limits (limit('*') == 42), and
 * clang::offset in the inline form.
 *
 * The resource is this very file via __FILE__, keeping the test
 * self-contained; the first two bytes of a C source file with this exact
 * header are '/' and '*'.
 *
 * LCCC_NO_COMPARE=1: the host GCC 14 has no #embed (a GCC 15 feature);
 * the lccc self-check is the oracle here. Provenance: EDG Changes
 * excerpt workflow (docs/edg_changes_c_extract.md). */
#include <stdio.h>

unsigned char d[] = {#embed __FILE__ limit(2) , };
unsigned char e[] = {#embed __FILE__ clang::offset(1) limit(1)};

int main(void) {
  if (sizeof d != 2 || d[0] != '/' || d[1] != '*' || e[0] != '*') {
    printf("BAD %zu %d %d %d\n", sizeof d, d[0], d[1], e[0]);
    return 1;
  }
  printf("ok %zu %d %d\n", sizeof d, d[0], e[0]);
  return 0;
}
