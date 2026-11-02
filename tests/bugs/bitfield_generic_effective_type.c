/* LCCC vs GCC divergence: _Generic controlling type for bit-fields.
 *
 * GCC (as documented in EDG Changes 2023-08-29, EDGcpfe/26614) treats a
 * bit field whose width exactly matches a smaller integer type AS that
 * type for _Generic selection: `unsigned u:8` -> unsigned char,
 * `int v:16` -> short. LCCC currently uses the declared type instead.
 *
 * Expected (GCC):   uchar short int
 * Observed (lccc):  uint int ll
 *
 * This is the E6 (bit-field ABI/semantics differential) work item's first
 * pinned divergence. */
#include <stdio.h>

struct {
  unsigned u : 8;
  int v : 16;
  long long w : 32;
} s;

int main(void) {
  printf("%s %s %s\n",
         _Generic((s.u), unsigned char : "uchar", unsigned int : "uint",
                  default : "unknown"),
         _Generic((s.v), short : "short", int : "int", default : "unknown"),
         _Generic((s.w), int : "int", long long : "ll", default : "unknown"));
  return 0;
}
