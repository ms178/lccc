/* C23 `#embed` grammar battery (EDG-transplant red-team audit):
 *   1. TWO inline embeds on ONE line (`{#embed ...}, {#embed ...}`) —
 *      the splice must handle several occurrences left to right and keep
 *      the trailing `};` of each (C23 6.10.15 mid-line form);
 *   2. clang::offset(K) combined with limit(N): bytes K..K+N-1;
 *   3. hex pp-number limit: limit(0x10);
 *   4. prefix/suffix token sequences around the byte list;
 *   5. if_empty on an exhausted slice (offset beyond EOF).
 * All expected values derive from the fixture edg_embed_bytes.bin (byte
 * values 0..255 in order). LCCC_NO_COMPARE=1: host GCC 14 cannot parse
 * #embed. Provenance: EDG Changes excerpt workflow
 * (docs/edg_changes_c_extract.md). */
#include <stdio.h>

static const unsigned char a[] = {#embed "edg_embed_bytes.bin" limit(0x4)}, b[] = {#embed "edg_embed_bytes.bin" clang::offset(250)};
static const unsigned char c[] = {#embed "edg_embed_bytes.bin" clang::offset(250) limit(3)};
static const int d[] = {#embed "edg_embed_bytes.bin" limit(2) prefix(1000,) suffix(,2000)};
static const int e[] = {#embed "edg_embed_bytes.bin" clang::offset(300) if_empty(7, 8)};

int main(void) {
  int ok = 1;
  ok &= sizeof a == 4 && a[0] == 0 && a[3] == 3;
  ok &= sizeof b == 6 && b[0] == 250 && b[5] == 255;
  ok &= sizeof c == 3 && c[0] == 250 && c[2] == 252;
  ok &= sizeof d == 4 * sizeof(int) && d[0] == 1000 && d[1] == 0 && d[2] == 1 &&
        d[3] == 2000;
  ok &= sizeof e == 2 * sizeof(int) && e[0] == 7 && e[1] == 8;
  if (!ok) {
    printf("BAD %zu %zu %zu %zu %zu\n", sizeof a, sizeof b, sizeof c,
           sizeof d, sizeof e);
    return 1;
  }
  printf("ok\n");
  return 0;
}
