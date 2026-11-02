/* GCC torture execute/20030613-1.c (PR optimization/10955), verbatim.
 *
 * LCCC regression: the peephole's loop-invariant GPR-load hoist only
 * recognised movq/movl/movb/movw stores when proving the slot stable, so
 * the in-loop `movdqu` struct copy clobbering 136(%rsp) was missed and the
 * `movq 136(%rsp), %r11` was hoisted above the loop, reading iteration 0's
 * value on every iteration. Fails at -O1 without the shared
 * frame_slot_stable_in_range engine. */


/* This used to fail on SPARC32 at -O3 because the loop unroller
   wrongly thought it could eliminate a pseudo in a loop, while
   the pseudo was used outside the loop.  */

extern void abort(void);

#define COMPLEX struct CS

COMPLEX {
  long x;
  long y;
};


static COMPLEX CCID (COMPLEX x)
{
  COMPLEX a;

  a.x = x.x;
  a.y = x.y;

  return a;
}


static COMPLEX CPOW (COMPLEX x, int y)
{
  COMPLEX a;
  a = x;

  while (--y > 0)
    a=CCID(a);

  return a;
}


static int c5p (COMPLEX x)
{
  COMPLEX a,b;
  a = CPOW (x, 2);
  b = CCID( CPOW(a,2) );

  return (b.x == b.y);
}


int main (void)
{
  COMPLEX  x;

  x.x = -7;
  x.y = -7;

  if (!c5p(x))
    abort();

  return 0;
}
