/* `x - a` compared with `b` through a dead copy may only be re-associated to
 * `x CMP a+b` for ZF/SF readers: CF/OF of the two subtractions differ
 * whenever `x - a` wraps.  The i686 peephole fused every condition, so the
 * range check below became `cmpl $127, y; ja` (y >u 127) and aborted for
 * every negative y (gcc.c-torture pr45034 at -O2/-O3/-Os). */
__attribute__((noinline)) void check_s8(int y)
{
  if (y < -128 || y > 127)
    __builtin_abort();
}

__attribute__((noinline)) int is_digit(int c)
{
  return (unsigned)(c - '0') <= 9u;
}

__attribute__((noinline)) int is_lower(int c)
{
  if ((unsigned)(c - 'a') > 25u)
    return 0;
  return 1;
}

int main(void)
{
  int digits = 0, lowers = 0;
  for (int y = -128; y <= 127; y++)
    check_s8(y);
  for (int c = -300; c < 300; c++) {
    digits += is_digit(c);
    lowers += is_lower(c);
  }
  return digits != 10 || lowers != 26;
}
