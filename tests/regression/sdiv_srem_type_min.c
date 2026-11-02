/* Signed division and remainder by the type's most negative value.
 *
 * div_by_const lowers `x / MIN` to `x == MIN` and `x % MIN` to
 * `x == MIN ? 0 : x` instead of an IDIV (every other dividend has a smaller
 * magnitude than MIN).  The checks pin both widths, the MIN dividend itself
 * (quotient 1, remainder 0, neither traps), its neighbours, and the I64
 * division by INT_MIN, which is an ordinary negative power of two and must
 * keep the shift path.  Shared with the -m32 variant, where the I64 forms
 * are register pairs. */
#include <limits.h>
#include <stdio.h>

static int fail;

#define CHECK(expr, want)                                                  \
	do {                                                               \
		long long got_ = (long long)(expr), want_ = (long long)(want); \
		if (got_ != want_) {                                       \
			printf("FAIL %s: %lld != %lld\n", #expr, got_, want_); \
			fail = 1;                                          \
		}                                                          \
	} while (0)

__attribute__((noinline)) static int d32(int x) { return x / INT_MIN; }
__attribute__((noinline)) static int r32(int x) { return x % INT_MIN; }
__attribute__((noinline)) static long long d64(long long x) { return x / LLONG_MIN; }
__attribute__((noinline)) static long long r64(long long x) { return x % LLONG_MIN; }
__attribute__((noinline)) static long long d64i(long long x) { return x / (long long)INT_MIN; }
__attribute__((noinline)) static long long r64i(long long x) { return x % (long long)INT_MIN; }
/* The quotient feeding arithmetic (a 0/1 compare result widened to I64). */
__attribute__((noinline)) static long long d64acc(long long x, long long y)
{
	return y * 3 + x / LLONG_MIN;
}

int main(void)
{
	static const int v32[] = {INT_MIN, INT_MIN + 1, INT_MIN + 2, -65536, -2, -1,
				  0, 1, 2, 65536, INT_MAX - 1, INT_MAX};
	static const long long v64[] = {LLONG_MIN, LLONG_MIN + 1, (long long)INT_MIN - 1,
					(long long)INT_MIN, (long long)INT_MIN + 1, -1, 0, 1,
					(long long)INT_MAX, (long long)INT_MAX + 1,
					(long long)UINT_MAX, LLONG_MAX - 1, LLONG_MAX};
	unsigned long long h = 1469598103934665603ULL;
	for (unsigned i = 0; i < sizeof v32 / sizeof v32[0]; i++) {
		int x = v32[i];
		CHECK(d32(x), x == INT_MIN);
		CHECK(r32(x), x == INT_MIN ? 0 : x);
		h = (h ^ (unsigned)d32(x)) * 1099511628211ULL;
		h = (h ^ (unsigned)r32(x)) * 1099511628211ULL;
	}
	for (unsigned i = 0; i < sizeof v64 / sizeof v64[0]; i++) {
		long long x = v64[i];
		long long qi = x / 2147483648LL; /* reference: positive divisor */
		CHECK(d64(x), x == LLONG_MIN);
		CHECK(r64(x), x == LLONG_MIN ? 0 : x);
		CHECK(d64i(x), -qi);
		CHECK(r64i(x), x - qi * 2147483648LL);
		CHECK(d64acc(x, (long long)i), 3LL * i + (x == LLONG_MIN));
		h = (h ^ (unsigned long long)d64(x)) * 1099511628211ULL;
		h = (h ^ (unsigned long long)r64(x)) * 1099511628211ULL;
		h = (h ^ (unsigned long long)d64i(x)) * 1099511628211ULL;
		h = (h ^ (unsigned long long)r64i(x)) * 1099511628211ULL;
	}
	printf("type_min div/rem hash %016llx\n", h);
	return fail;
}
