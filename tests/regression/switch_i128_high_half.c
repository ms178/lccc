/* 128-bit switch: a case value representable in i64 matches only when the
 * high half of the switch expression is the sign extension of that case
 * value (0 for non-negative cases, -1 for negative ones).  The compare
 * chain operates on the low 64-bit half, so ((__int128)1 << 64) must NOT
 * match `case 0` (gcc.c-torture execute/pr122943.c). */
extern void abort(void);

__attribute__((noipa)) static unsigned char baz(__int128 val)
{
  unsigned char result = 0;
  switch (val)
    {
    case 0: result = 1; break;
    case 1: result = 2; break;
    case 2: result = 3; break;
    default: break;
    }
  return result;
}

__attribute__((noipa)) static unsigned char mixed(__int128 val)
{
  unsigned char result = 0;
  switch (val)
    {
    case -1: result = 1; break;
    case 1: result = 2; break;
    case 2: result = 3; break;
    default: break;
    }
  return result;
}

int main(void)
{
  __int128 one_shift_64 = (__int128)1 << 64;
  if (baz(0) != 1)
    abort();
  if (baz(1) != 2)
    abort();
  if (baz(2) != 3)
    abort();
  if (baz(-1) != 0)
    abort();
  if (baz(one_shift_64) != 0)
    abort();
  if (baz(-one_shift_64) != 0)
    abort();
  if (baz(((__int128)1 << 64) + 1) != 0)
    abort();
  if (baz(((__int128)-1 << 64) | 1) != 0) /* high=-1, low=1: no case */
    abort();
  if (mixed(-1) != 1)
    abort();
  if (mixed(1) != 2)
    abort();
  if (mixed(2) != 3)
    abort();
  if (mixed(one_shift_64) != 0)
    abort();
  if (mixed(0) != 0)
    abort();
  return 0;
}
