/*
 * Cross-block integer-Mul CSE x dot-product interaction pin (end-to-end).
 *
 * The loop carries BOTH:
 *   1. a VARIANT multiply `i * i` in the loop-HEADER exit compare — the
 *      canonical definition, dominated-blocks-only — which the body's
 *      `t += i * i` duplicates: the operands are the loop IV, so LICM
 *      cannot hoist either copy and if_convert cannot fold the loop
 *      header; only the cross-block GVN rename collapses the body
 *      duplicate onto the header's canonical (the PR #611 optimization),
 *      and
 *   2. a dot-product multiply `a[i] * b[i]` feeding an accumulator
 *      through body-local loads — the vectorizer's structural
 *      requirement. It must NOT be renamed away: its operands are loads
 *      whose value numbers only exist inside the loop (GVN invalidates
 *      load entries at the header merge point), so no dominator table
 *      entry can ever match it.
 *
 * Expected post-GVN loop (verified by check_gvn_cross_block_mul_dot.sh):
 *   header: imul (i*i, the exit compare) — the ONE canonical variant Mul
 *   body:   imul (mem), (mem) — the dot-product Mul, memory-form operands
 * With cross-block CSE disabled the body re-materializes `i * i` (and the
 * exit compare at the latch), which the gate detects as extra
 * register-register imuls inside the loop body.
 */
long
dot_and_variant(const long *restrict a, const long *restrict b, long n)
{
  long s = 0;
  long t = 0;

  for (long i = 2; i * i < n; i++) {
    s += a[i] * b[i]; /* body-local dot-product Mul — never renamed */
    t += i * i;       /* dominated duplicate — renamed onto the header's */
  }
  return s + t;
}

#include <stdio.h>

int
main(void)
{
  enum { N = 257 };
  static long a[N], b[N];

  /* Deterministic non-trivial operands (odd/even mix keeps both loop
     paths of the original guarded variant exercised by future edits). */
  for (long i = 0; i < N; i++) {
    a[i] = (i * 7) % 19 - 9;
    b[i] = (i * 11) % 23 - 11;
  }

  long ref_s = 0, ref_t = 0;
  for (long i = 2; i * i < N; i++) {
    ref_s += a[i] * b[i];
    ref_t += i * i;
  }
  long ref = ref_s + ref_t;

  long got = dot_and_variant(a, b, N);
  if (got != ref) {
    printf("MISMATCH dot_and_variant: got %ld want %ld\n", got, ref);
    return 1;
  }
  puts("OK gvn cross-block mul dot product");
  return 0;
}
