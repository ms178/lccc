/* Aggregate-split fire probe. `aggregate_sroa` form 4 splits `V t` (16 bytes,
 * 4 constant-offset fields) into scalars:
 *   CCC_AGG_SPLIT_DEBUG=1 ./target/fastbuild/lccc -O2 artifacts/repros/split_fire_probe.c
 * prints `[SROA-split] fn f alloca v3 size=16 fields=4`; with
 * CCC_NO_AGGREGATE_SPLIT=1 it prints nothing. Both builds print 75634688, which
 * is what gcc -O2 prints. The form is default-on since f43037f9's series, so
 * this is the one-line check that the escape hatch still works.
 */
#include <stdio.h>
typedef struct { unsigned a, b, c, d; } V;
static unsigned scratch[4];
__attribute__((noinline)) unsigned f(V v, unsigned k) {
  V t;
  t.a = v.a + 1u; t.b = v.b ^ k; t.c = t.a + t.b; t.d = t.c * 3u;
  scratch[0] = t.d; scratch[1] = t.a ^ t.c; scratch[2] = t.b + 7u; scratch[3] = t.d - 1u;
  return scratch[0] + scratch[1] + scratch[2] + scratch[3] + t.c;
}
int main(void) { V v = {3u, 11u, 7u, 5u}; unsigned long acc = 0;
  for (int i = 0; i < 4096; i++) acc += f(v, (unsigned)i);
  printf("%lu\n", acc); return acc == 0; }
