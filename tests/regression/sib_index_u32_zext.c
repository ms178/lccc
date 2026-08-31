/* S11 regression pin (O2): unsigned narrow SIB indices need in-place ZERO
 * extension (`movl %rNd, %rNd` — the 64-bit destination form is not
 * encodable and would be rejected by the assembler). After an I32/U32
 * call only the low 32 bits of the return register are defined, so the
 * folded index `arr[three() + 1]` must extend from the low half by the
 * index type's semantics before the SIB reads the full register.
 * Expected exit 0. */

unsigned int arr[8];

static unsigned int
three (void)
{
  volatile unsigned int x = 3;
  return x;
}

int
main (void)
{
  arr[three () + 1] = 7u;
  if (arr[4] != 7u)
    return 1;
  if (arr[0] != 0u || arr[3] != 0u)
    return 1;
  return 0;
}
