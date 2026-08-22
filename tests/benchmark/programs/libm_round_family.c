/*
 * Workload-derived kernel: glibc 2.44 libm scalar rounding entry points.
 *
 * glibc's generic C implementations (sysdeps/ieee754/dbl-64/s_floor.c,
 * s_trunc.c, s_rint.c, ...) define floor()/trunc()/rint()/nearbyint()/
 * roundeven() AS the corresponding GCC builtin under USE_*_BUILTIN, so the
 * quality of a libm built by this compiler is EXACTLY the quality of the
 * inline expansion measured here: one vroundsd (imm 9/11/4/12/8) plus the
 * call frame. A compiler that lowers these to libm calls self-recurses
 * inside glibc and pays a PLT round-trip everywhere else.
 *
 * The kernel also exercises copysign (pure SSE bit ops; a call would
 * recurse inside glibc's s_copysign.c) and the fma contraction path.
 * Value checks fold the results so the loop cannot be dead-code eliminated,
 * and the rotating input vector defeats constant folding.
 */
#include <stdio.h>

/* Self-contained at ANY -march: when a compiler does not inline a rounding
 * builtin, the call binds to these local definitions instead of libm (no
 * -lm at link). They are deliberately NOT written as `return __builtin_X`:
 * a non-inlining compiler lowers that to a call to X — i.e. to ITSELF
 * (infinite recursion; the same trap glibc's s_floor.c poses to a compiler
 * without inline expansion). The 2^52 magic-number trick rounds ties-to-
 * even exactly like ROUNDSD for every |x| < 2^52, so results are
 * bit-identical to the inline vroundsd path on this benchmark's domain.
 */
static double rte (double x) /* rint/nearbyint/roundeven, |x| < 2^52 */
{
  const double big = 4503599627370496.0; /* 2^52 */
  return x >= 0.0 ? (x + big) - big : (x - big) + big;
}
double rint (double x) { return rte (x); }
double nearbyint (double x) { return rte (x); }
double roundeven (double x) { return rte (x); }
double floor (double x) { double t = rte (x); return t > x ? t - 1.0 : t; }
double ceil (double x) { double t = rte (x); return t < x ? t + 1.0 : t; }
double trunc (double x) { double t = rte (x);
  if (x >= 0.0) return t > x ? t - 1.0 : t;
  return t < x ? t + 1.0 : t; }
double copysign (double x, double y)
{
  union { double d; unsigned long long u; } a = { x }, b = { y };
  a.u = (a.u & 0x7fffffffffffffffULL) | (b.u & 0x8000000000000000ULL);
  return a.d;
}
/* fma fallback: on this benchmark's operands (integer-valued f times 0.5
 * plus a small integer) every intermediate is exact, so mul+add equals the
 * fused result bit-for-bit. */
double fma (double x, double y, double z) { return x * y + z; }

#define N 4096
#define PASSES 20000U

static double buf[N];
static double out[N];

__attribute__((noinline)) static double
round_family_pass (void)
{
  double acc = 0.0;
  for (int i = 0; i < N; i++)
    {
      double x = buf[i];
      /* Eight directed-rounding ops per element: the vroundsd latency chain
         and the andpd/orpd copysign pair dominate; a libm-call compiler
         pays 8 PLT calls per element instead.  */
      double f = __builtin_floor (x);
      double c = __builtin_ceil (x);
      double t = __builtin_trunc (x);
      double r = __builtin_rint (x);
      double n = __builtin_nearbyint (x);
      double e = __builtin_roundeven (x);
      double s = __builtin_copysign (t, x);
      double m = __builtin_fma (f, 0.5, c);
      out[i] = m + r + n + e + s;
      acc += out[i] - t;
    }
  return acc;
}

int
main (void)
{
  /* Rotating non-integral values, positive and negative, spanning the
     ties-to-even interesting range.  */
  for (int i = 0; i < N; i++)
    buf[i] = (i - N / 2) * 0.37519 + ((i & 3) * 0.25);

  double total = 0.0;
  for (unsigned p = 0; p < PASSES; p++)
    {
      total += round_family_pass ();
      /* Rotate signs so branchless copysign sees both directions. */
      buf[p % N] = -buf[p % N];
    }
  printf ("libm-round-family: %.6f\n", total);
  return 0;
}
