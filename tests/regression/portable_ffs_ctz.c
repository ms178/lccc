/*
 * Portable Linux-style __ffs tree: this is a correctness oracle for the
 * bit-idiom recognizer, not a test-specific expected assembly.
 *
 * The zero input is intentional. This exact Linux tree returns 63 for zero;
 * any lowering to a zero-undefined bsf sequence or to raw tzcnt is invalid.
 * The optimizer may replace the tree with native Ctz only when it preserves
 * that result.
 */
__attribute__((noinline)) unsigned int
portable_ffs64(unsigned long word)
{
  unsigned int num = 0;
  if ((word & 0xffffffffUL) == 0) { num += 32; word >>= 32; }
  if ((word & 0xffffUL) == 0) { num += 16; word >>= 16; }
  if ((word & 0xffUL) == 0) { num += 8; word >>= 8; }
  if ((word & 0xfUL) == 0) { num += 4; word >>= 4; }
  if ((word & 0x3UL) == 0) { num += 2; word >>= 2; }
  if ((word & 0x1UL) == 0) num += 1;
  return num;
}

int
main(void)
{
  static const unsigned long values[] = {
    0UL, 1UL, 2UL, 4UL, 8UL, 0x100UL, 0x10000UL,
    0x80000000UL, 0x100000000UL, 0x8000000000000000UL,
    0x0101010101010100UL, 0xaaaaaaaaaaaaaa00UL,
  };
  static const unsigned int expected[] = {
    63, 0, 1, 2, 3, 8, 16, 31, 32, 63, 8, 9,
  };
  unsigned int i;
  for (i = 0; i < sizeof(values) / sizeof(values[0]); ++i)
    if (portable_ffs64(values[i]) != expected[i])
      return (int)i + 1;
  return 0;
}
