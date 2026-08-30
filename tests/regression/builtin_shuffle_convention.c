// Regression pin (all opt levels): __builtin_shuffle (GCC vector extension).
//
// pr94591 shape: shuffle with an identity mask through a vector-type cast
// compound literal; pr85331 shape: shuffle with a constant reverse mask.
// Covers the return-convention contract: <=8-byte vectors travel as packed
// register values (the vector-assignment consumer spills them), >8-byte
// vectors by memory (the result alloca pointer). A convention mismatch
// corrupts the destination with the alloca's ADDRESS (amd64: si={ptr_lo,
// ptr_hi}).
extern void abort (void);

typedef unsigned V2SI_u __attribute__((vector_size(8)));
typedef int V2SI_d __attribute__((vector_size(8)));
typedef unsigned long V2DI_u __attribute__((vector_size(16)));
typedef long V2DI_d __attribute__((vector_size(16)));
typedef double V2DF __attribute__((vector_size(16)));

void
id_v2si (V2SI_d *v)
{
  *v = __builtin_shuffle (*v, (V2SI_d) (V2SI_u)
			  { 0, 1 });
}

void
id_v2di (V2DI_d *v)
{
  *v = __builtin_shuffle (*v, (V2DI_d) (V2DI_u)
			  { 0, 1 });
}

void
rev_v2df (V2DF *r)
{
  V2DF y = { 1.0, 2.0 };
  V2DI_u m = { 1UL, 0UL };
  *r = __builtin_shuffle (y, (V2DI_d) m);
}

int
main (void)
{
  V2SI_d si = { 35, 42 };
  V2DI_d di = { 63, 38 };
  V2DF r;
  id_v2si (&si);
  id_v2di (&di);
  rev_v2df (&r);
  if (si[0] != 35 || si[1] != 42)
    abort ();
  if (di[0] != 63 || di[1] != 38)
    abort ();
  if (r[0] != 2.0 || r[1] != 1.0)
    abort ();
  return 0;
}
