// Regression pin (O1): extern-inline bodies must be inlined at -O1 and their
// __builtin_constant_p queries resolved to 1 after argument specialization,
// with the not-constant fallback arm (undef) folded away and deleted. If the
// inliner, the IsConstant resolver, or the post-resolution CFG prune
// regresses, `undef` stays referenced and the link step fails.
extern void undef (void);
extern void exit (int);

void bar (unsigned x) { }
void baz (unsigned x) { }

extern inline void foo (int a, int b)
{
  int c = 0;
  while (c++ < b)
    (__builtin_constant_p (a) ? ((a) > 20000 ? undef () : bar (a)) : baz (a));
}

int main (void)
{
  foo (10, 100);
  exit (0);
}
