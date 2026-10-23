/* -fmath-errno sqrt/sqrtf (C17 7.12.7.5, glibc MATH_ERRNO): a negative
 * operand must set errno = EDOM; -0, +inf, NaN and every non-negative
 * operand must not.  GCC 16.2 and Clang 23.1.3 inline the instruction and
 * call the library only on the ordered `x < 0` slow path.
 *
 * The .flags bind `sqrt`/`sqrtf` to the test_* definitions at link time, so
 * the slow path returns a value the inline instruction never produces and
 * counts its calls:
 *   - the slow call's RESULT must reach its user (the x86-64 cold-call XMM
 *     split once restored the caller-saved home over it -> NaN sum);
 *   - loop accumulators live across the cold call must survive it;
 *   - provably non-negative operands (sum of squares) never take it.
 * The slow-call counts assume the inline fast path of the .flags tier (-O2,
 * where GCC, Clang and LCCC agree byte-for-byte); at -O0 every call is a
 * library call by design. */
#include <errno.h>
#include <math.h>
#include <stdio.h>

/* volatile: compilers model library sqrt as writing only errno. */
static volatile int slow_calls;

/* No libm here: `sqrt` IS this function after the link-time binding, so a
 * library or builtin call would recurse (every call at -O0; NaN operands
 * under Clang, whose unordered-compare slow path is also legal). */
static double newton(double x) {
  if (x == 0.0 || x != x || x > 1.7e308) return x;
  double g = x > 1.0 ? x : 1.0;
  for (int i = 0; i < 200; i++) g = 0.5 * (g + x / g);
  return g;
}

double test_sqrt(double x) {
  slow_calls++;
  if (x >= 0.0 || x != x) return newton(x);
  errno = EDOM;
  return 1000.0 - x;
}

float test_sqrtf(float x) {
  slow_calls++;
  if (x >= 0.0f || x != x) return (float)newton(x);
  errno = EDOM;
  return 500.0f - x;
}

__attribute__((noinline)) double acc(const double *a, int n) {
  double s = 0.0, m = 1.0;
  for (int i = 0; i < n; i++) {
    double r = sqrt(a[i]);
    s += r;
    m = m * 0.5 + r;
  }
  return s + m;
}

__attribute__((noinline)) float accf(const float *a, int n) {
  float s = 0.0f;
  for (int i = 0; i < n; i++) s += sqrtf(a[i]) * (float)(i + 1);
  return s;
}

__attribute__((noinline)) double dist(const double *x, const double *y, int n) {
  double s = 0.0;
  for (int i = 0; i < n; i++) s += sqrt(x[i] * x[i] + y[i] * y[i]);
  return s;
}

__attribute__((noinline)) int edom(double x) {
  errno = 0;
  volatile double r = sqrt(x);
  (void)r;
  return errno == EDOM;
}

int main(void) {
  double a[8] = {4.0, -1.0, 9.0, -2.0, 16.0, 0.25, -0.5, 1.0};
  float f[6] = {1.0f, -4.0f, 2.25f, -0.0f, 9.0f, -1.0f};
  double x[5] = {3.0, -6.0, 0.0, -1e200, 1e-200};
  double y[5] = {4.0, 8.0, -0.0, 1e200, -1e-200};
  int fails = 0;

  errno = 0;
  slow_calls = 0;
  double r = acc(a, 8);
  printf("acc %.6f errno=%d slow=%d\n", r, errno == EDOM, slow_calls);
  fails += r != 3594.25390625 || errno != EDOM || slow_calls != 3;

  errno = 0;
  slow_calls = 0;
  float rf = accf(f, 6);
  printf("accf %.4f errno=%d slow=%d\n", (double)rf, errno == EDOM, slow_calls);
  fails += rf != 4034.5f || errno != EDOM || slow_calls != 2;

  errno = 0;
  slow_calls = 0;
  double d = dist(x, y, 5);
  printf("dist %.6g errno=%d slow=%d\n", d, errno, slow_calls);
  fails += errno != 0 || slow_calls != 0;

  static const double ops[] = {-1.0, -0.0, 0.0, 2.0, -INFINITY, INFINITY, NAN, -NAN};
  static const int want[] = {1, 0, 0, 0, 1, 0, 0, 0};
  for (int i = 0; i < 8; i++) {
    int e = edom(ops[i]);
    printf("edom[%d]=%d\n", i, e);
    fails += e != want[i];
  }
  printf("%s\n", fails ? "FAIL" : "OK");
  return fails != 0;
}
