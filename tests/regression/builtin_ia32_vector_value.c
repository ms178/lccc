/* Raw GCC __builtin_ia32_* vector builtins return the vector BY VALUE.
 *
 * The kernel's NAP cpuidle governor (a CachyMod patch) uses GCC vector
 * extensions directly — kernel code cannot include userspace intrin
 * headers:
 *     typedef float v4sf __attribute__((__vector_size__(16)));
 *     return __builtin_ia32_maxps(__builtin_ia32_minps(v, hi), lo);
 *
 * Two distinct wrong-code bugs lived here:
 *  1. The lowering returned the result ALLOCA POINTER (the _mm_* wrappers
 *     dereference it in the header; a raw builtin caller does not), so the
 *     caller received a stack address instead of 16 data bytes.
 *  2. The builtin's result type defaulted to I64, so the return path
 *     sign-extended half the vector away (cqto), zeroing lanes 2-3.
 *
 * Checked at -O0 and -O2 via .flags? No — one binary, but with a noinline
 * function boundary (exercises the I128 return ABI) AND an inlined use
 * (exercises direct value flow), against golden lane values.
 */
#include <stdio.h>

typedef float v4sf __attribute__((__vector_size__(16)));

__attribute__((noinline))
static v4sf clamp_noinline(v4sf v, v4sf lo, v4sf hi)
{
	return __builtin_ia32_maxps(__builtin_ia32_minps(v, hi), lo);
}

static inline v4sf clamp_inline(v4sf v, v4sf lo, v4sf hi)
{
	return __builtin_ia32_maxps(__builtin_ia32_minps(v, hi), lo);
}

int main(void)
{
	volatile float a0 = 5.0f, a1 = -3.0f, a2 = 10.0f, a3 = 0.5f;
	v4sf v = { a0, a1, a2, a3 };
	v4sf lo = { 0.0f, 0.0f, 0.0f, 0.0f };
	v4sf hi = { 4.0f, 4.0f, 4.0f, 4.0f };

	v4sf r1 = clamp_noinline(v, lo, hi);
	if (r1[0] != 4.0f || r1[1] != 0.0f || r1[2] != 4.0f || r1[3] != 0.5f) {
		printf("FAIL noinline %f %f %f %f\n", r1[0], r1[1], r1[2], r1[3]);
		return 1;
	}

	v4sf r2 = clamp_inline(v, lo, hi);
	if (r2[0] != 4.0f || r2[1] != 0.0f || r2[2] != 4.0f || r2[3] != 0.5f) {
		printf("FAIL inline %f %f %f %f\n", r2[0], r2[1], r2[2], r2[3]);
		return 2;
	}

	/* min/max asymmetry: prove operand ORDER is right (minps/maxps are
	 * not commutative for NaN, but here we just check lane routing). */
	v4sf a = { 1.0f, 8.0f, 1.0f, 8.0f };
	v4sf b = { 8.0f, 1.0f, 8.0f, 1.0f };
	v4sf mn = __builtin_ia32_minps(a, b);
	v4sf mx = __builtin_ia32_maxps(a, b);
	if (mn[0] != 1.0f || mn[1] != 1.0f || mx[0] != 8.0f || mx[1] != 8.0f) {
		printf("FAIL minmax\n");
		return 3;
	}

	printf("PASS builtin_ia32_vector_value\n");
	return 0;
}
