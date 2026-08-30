/* __builtin_apply/__builtin_apply_args/__builtin_return: the forwarding
 * triangle.  apply_args snapshots the current function's incoming arguments
 * (integer registers + al + XMM argument registers on x86-64; the register
 * block plus the caller's stack argument area on i686), apply re-delivers
 * them to the target and captures its return value into a result block, and
 * builtin_return returns that value from the current function.  A correct
 * implementation makes forward() behave exactly like add3().  (Modeled on
 * gcc.c-torture execute/pr47237.c plus the value-returning path.) */
extern void abort(void);

static int add3(int a, int b, int c)
{
  return a + b * 2 + c * 3;
}

static int forward(int a, int b, int c)
{
  void *r = __builtin_apply(add3, __builtin_apply_args(), 24);
  __builtin_return(r);
}

static int forward_zero(int a, int b, int c)
{
  void *r = __builtin_apply(add3, __builtin_apply_args(), 24);
  __builtin_return(r);
}

int main(void)
{
  if (forward(5, 6, 7) != 38)
    abort();
  if (forward(1, 0, 0) != 1)
    abort();
  if (forward_zero(-3, 4, 10) != -3 + 8 + 30)
    abort();
  return 0;
}
