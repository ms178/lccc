// Regression pin (both backends, all levels): vector_size may take an
// expression involving sizeof of a CHAINED typedef, per GCC vector
// extensions used pervasively in the torture suite (pr123625 et al).
// The parse-time constant evaluator must walk the typedef chain to its
// base spec; if the size expression stays unevaluable the attribute is
// dropped and the vector object collapses to its 8-byte element type
// (stores overrun the object, indexing dereferences element values).
#include <stdint.h>
#define BS_VEC(type, num) type __attribute__((vector_size(num * sizeof(type))))
typedef long long base_t;
typedef base_t mid_t;
typedef mid_t leaf_t;

int main (void)
{
  BS_VEC (leaf_t, 16) v = { 1, 8096386231136, 9039249955151 };
  if (sizeof (v) != 128)
    return 1;
  if (v[1] != 8096386231136 || v[2] != 9039249955151)
    return 2;
  long long s = 0;
  for (uint32_t i = 0; i < 16; i++)
    s += v[i];
  if (s != 17135636186288)
    return 3;
  return 0;
}
