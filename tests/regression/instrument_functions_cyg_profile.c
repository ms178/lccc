/* S13 regression pin (all levels): -finstrument-functions.
 * The IR-level instrumentation pass must call __cyg_profile_func_enter
 * after entry and __cyg_profile_func_exit before every return of every
 * defined function that does not carry no_instrument_function — including
 * when the attribute lives ONLY on a prior file-scope declaration (GCC
 * merges declaration attributes into the definition; gcc.c-torture
 * eeprof-1 writes `int main () NOCHK;` with a bare definition). The hooks
 * themselves are never instrumented (unbounded recursion otherwise).
 * Expected exit 0. */
#include <stdio.h>

#define ASSERT(X) \
  if (!(X)) { printf("FAIL %d\n", __LINE__); abort(); }
#define NOCHK __attribute__ ((no_instrument_function))

extern void abort (void);

int entry_calls, exit_calls;
void (*last_fn_entered) (void);
void (*last_fn_exited) (void);

__attribute__ ((noinline)) int main (void) NOCHK;

__attribute__ ((noinline)) void
foo (void)
{
  ASSERT (last_fn_entered == foo);
}

__attribute__ ((noinline)) static void
foo2 (void)
{
  ASSERT (entry_calls == 1 && exit_calls == 0);
  ASSERT (last_fn_entered == foo2);
  foo ();
  ASSERT (entry_calls == 2 && exit_calls == 1);
  ASSERT (last_fn_entered == foo);
  ASSERT (last_fn_exited == foo);
}

__attribute__ ((noinline)) void nfoo (void) NOCHK;
__attribute__ ((noinline)) void
nfoo (void)
{
  ASSERT (entry_calls == 2 && exit_calls == 2);
}

int
main (void)
{
  ASSERT (entry_calls == 0 && exit_calls == 0);
  foo2 ();
  ASSERT (entry_calls == 2 && exit_calls == 2);
  ASSERT (last_fn_exited == foo2);
  nfoo ();
  ASSERT (entry_calls == 2 && exit_calls == 2);
  return 0;
}

void __cyg_profile_func_enter (void *, void *) NOCHK;
void __cyg_profile_func_exit (void *, void *) NOCHK;

__attribute__ ((noinline)) void
__cyg_profile_func_enter (void *fn, void *parent)
{
  (void) parent;
  entry_calls++;
  last_fn_entered = (void (*) (void)) fn;
}
__attribute__ ((noinline)) void
__cyg_profile_func_exit (void *fn, void *parent)
{
  (void) parent;
  exit_calls++;
  last_fn_exited = (void (*) (void)) fn;
}
