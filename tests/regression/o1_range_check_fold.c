// Regression pin (O1): range-check folding must run at -O1.
// gcc.c-torture 20041114-1: var <= 0 || (unsigned)(var - 1) < UINT_MAX is
// always true; if the fold is missing at -O1 the link_failure call survives.
#include <limits.h>

void link_failure (void);

volatile int v;

void foo (int var)
{
  if (!(var <= 0 || ((long unsigned) (unsigned) (var - 1) < UINT_MAX)))
    link_failure ();
}

int main (void)
{
  foo (v);
  return 0;
}
