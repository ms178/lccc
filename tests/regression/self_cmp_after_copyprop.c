// Regression pin (O1): identical-operand integer compares must fold once the
// shift identity Copies are propagated. gcc.c-torture shiftopt-1 shape: the
// compare becomes Cmp(Ne, v, v) only after copy propagation, so the fold has
// to live in the constant folder, not the front end. If the fold regresses,
// the call to the deliberately undefined link_error survives the link step.
extern void link_error (void);

int
stest (int x)
{
  if (x >> 0 != x)
    link_error ();
  if (x << 0 != x)
    link_error ();
  if (0 << x != 0)
    link_error ();
  if (0 >> x != 0)
    link_error ();
  if (-1 >> x != -1)
    link_error ();
  if (~0 >> x != ~0)
    link_error ();
  return x;
}

int
main (void)
{
  return stest (3) == 3 ? 0 : 1;
}
