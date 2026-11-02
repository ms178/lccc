/* S11 regression pin (O1/O2): the indexed-gep fold peels
 * `shl(add(iv, 1), 3)` down to the raw I32 call result and uses its home
 * register as a 64-bit SIB index. The register content after an I32 return
 * is only 32-bit-defined (System V names only %eax), so without the
 * in-place sign extension the folded store `k[baz()+1] = &b` computes a
 * wild address (gcc.c-torture pr110115: `16(%rsp,%r9,8)` with
 * %r9 = 0x00000000FFFFFFFF instead of the required sext -1).
 * Correct output: b stays 0 and the program exits 0. */

int a;
signed char b;

static int
foo (signed char *e, int f)
{
  int d;
  for (d = 0; d < f; d++)
    e[d] = 0;
  return d;
}

int
bar (signed char e, int f)
{
  signed char h[20];
  int i = foo (h, f);
  return i;
}

int
baz ()
{
  switch (a)
    {
    case 'f':
      return 0;
    default:
      return ~0;
    }
}

int
main ()
{
  {
    signed char *k[3];
    int d;
    for (d = 0; bar (8, 15) - 15 + d < 1; d++)
      k[baz () + 1] = &b;
    *k[0] = -*k[0];
  }
  return a + b;
}
