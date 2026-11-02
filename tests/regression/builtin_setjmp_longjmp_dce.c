/* __builtin_setjmp is returns-twice and __builtin_longjmp never returns.
 * Interprocedural purity analysis must classify both intrinsics as effectful:
 * when it treated them as purity-neutral, DCE deleted the call chain that
 * performs the longjmp, `a = 1` became dead relative to a deleted call, and
 * the function collapsed to an empty infinite loop (gcc.c-torture
 * execute/pr60003.c at -O1/-O2).  A correct build reaches the resume path
 * with `a = 1` still observable and returns x. */
extern void abort(void);

unsigned long long jmp_buf[5];

__attribute__((noinline, noclone)) void
baz (void)
{
  __builtin_longjmp (&jmp_buf, 1);
}

__attribute__((noinline, noclone)) void
bar (void)
{
  baz ();
}

__attribute__((noinline, noclone)) int
foo (int x)
{
  int a = 0;

  if (__builtin_setjmp (&jmp_buf) == 0)
    {
      while (1)
	{
	  a = 1;
	  bar ();
	}
    }
  else
    {
      if (a == 0)
	return 0;
      else
	return x;
    }
}

int
main ()
{
  if (foo (1) == 0)
    abort ();

  return 0;
}
