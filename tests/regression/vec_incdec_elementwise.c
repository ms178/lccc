// Regression pin: vector ++/-- must step by one ELEMENT per lane (GCC
// vector extensions: v-- == v = v - 1 with the scalar splatted).  The
// generic scalar/pointer inc/dec path stepped the whole object by
// sizeof(vector), corrupting every lane (pr123753: u.w-- produced
// 0xfff8 in lane 0 instead of 0xffff).
typedef int V __attribute__((__vector_size__ (8)));
typedef short W __attribute__((__vector_size__ (8)));
union { unsigned short u[4]; W w; } u;
V v;

V
foo ()
{
  u.w--;
  V r = v + u.u[0];
  return r;
}

int
main ()
{
  if (sizeof (int) != 4 || sizeof (short) != 2)
    return 0;
  V x = foo ();
  if (x[0] != (unsigned short) -1 || x[1] != (unsigned short) -1)
    __builtin_abort ();
  W t = (W){0, 0, 0, 0};
  W pre = --t;
  if (((unsigned short *) &t)[0] != 0xffff)
    __builtin_abort ();
  if (((unsigned short *) &pre)[0] != 0xffff)
    __builtin_abort ();
  return 0;
}
