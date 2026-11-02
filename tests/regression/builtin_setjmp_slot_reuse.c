/* Values defined before __builtin_setjmp and consumed on the resume path must
 * keep their stack slots: returns-twice functions may not pack stack slots by
 * plain-CFG liveness, because the resume edge from any call that longjmps is
 * invisible to that liveness.  At -O2 the tier-2 liveness packing handed the
 * .rodata "test" pointer's slot to the else-branch's loop bookkeeping value,
 * so after the longjmp landed strcmp compared against a clobbered pointer and
 * the test aborted (gcc.c-torture execute/built-in-setjmp.c). */
#include <stdlib.h>
#include <string.h>

void *buf[20];

__attribute__((noinline)) void
sub2 (void)
{
  __builtin_longjmp (buf, 1);
}

int
main ()
{
  char *p = (char *) __builtin_alloca (20);

  strcpy (p, "test");

  if (__builtin_setjmp (buf))
    {
      if (strcmp (p, "test") != 0)
	abort ();

      exit (0);
    }

  {
    int *q = (int *) __builtin_alloca (p[2] * sizeof (int));
    int i;

    for (i = 0; i < p[2]; i++)
      q[i] = 0;

    while (1)
      sub2 ();
  }
}
