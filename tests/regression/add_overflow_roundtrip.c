/* __builtin_add_overflow: the truncate/extend-back round-trip must observe
 * the truncated value, not the stale wide accumulator.  The peephole's
 * dead-sign-extension pass used to delete the cltq feeding the full-width
 * store of the extended-back value (it saw only the preceding narrow
 * movl %eax store), so the Ne compare compared the wide value against
 * itself and overflow was never reported.  Cases: constant and variable
 * wide operands, result narrower than the mathematical sum. */
extern void abort(void);

int u, v;

__attribute__((noipa)) static int ov_const(void)
{
  /* 8719476735 does not fit int: overflow must be reported */
  return __builtin_add_overflow(8719476735LL, u, &v);
}

__attribute__((noipa)) static int ov_var(long long w)
{
  return __builtin_add_overflow(w, u, &v);
}

__attribute__((noipa)) static int ov_fit(void)
{
  /* 2147483647 + 0 fits int exactly: no overflow */
  return __builtin_add_overflow(2147483647LL, u, &v);
}

int main(void)
{
  if (ov_const() != 1)
    abort();
  if (ov_var(8719476735LL) != 1)
    abort();
  if (ov_var(2147483647LL) != 0)
    abort();
  if (ov_fit() != 0)
    abort();
  return 0;
}
