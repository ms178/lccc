/* C23 `#embed` byte-oracle correctness (EDG-transplant red-team audit):
 * embed the 256-byte fixture edg_embed_bytes.bin (byte values 0..255 in
 * order) and verify EVERY byte survives the directive's decimal byte-list
 * expansion — FNV-1a hash of the embedded array plus sizeof, checked
 * against a constant derived from a GCC-compiled reference array of the
 * same bytes (differential oracle run recorded in the audit session:
 * both compilers produced 0x4242dc5249c33625 / 256).
 *
 * LCCC_NO_COMPARE=1: host GCC 14 cannot parse #embed itself; the oracle
 * constant below carries its verdict. Provenance: EDG Changes excerpt
 * workflow (docs/edg_changes_c_extract.md). */
#include <stdio.h>

static const unsigned char emb[] = {#embed "edg_embed_bytes.bin"};

int main(void) {
  unsigned long long h = 14695981039346656037ULL;
  for (unsigned i = 0; i < sizeof emb; i++) {
    h ^= emb[i];
    h *= 1099511628211ULL;
  }
  if (sizeof emb != 256 || h != 0x4242dc5249c33625ULL) {
    printf("BAD %zu %llx\n", sizeof emb, h);
    return 1;
  }
  /* Spot checks of values that exercise 1/2/3-digit emission. */
  if (emb[0] != 0 || emb[9] != 9 || emb[10] != 10 || emb[99] != 99 ||
      emb[100] != 100 || emb[255] != 255) {
    printf("BAD-BYTES %u %u %u %u %u %u\n", emb[0], emb[9], emb[10],
           emb[99], emb[100], emb[255]);
    return 1;
  }
  printf("ok %zu %llx\n", sizeof emb, h);
  return 0;
}
