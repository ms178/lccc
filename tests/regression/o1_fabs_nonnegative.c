// Regression pin (O1): fabs(x) < 0.0 is always false (NaN included); the
// comparison fold must apply at -O1 (gcc.c-torture 20020720-1 shape).
extern double fabs (double);
extern void link_error (void);

void foo (double x)
{
  double p = fabs (x);
  double q = 0.0;
  if (p < q)
    link_error ();
}

int main (void)
{
  foo (1.0);
  return 0;
}
