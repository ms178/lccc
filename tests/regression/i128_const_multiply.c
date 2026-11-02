/* Exhaustive coverage for the i128 constant-multiplier fast path.
 *
 * The backend specialises a 128-bit multiply by a compile-time constant into
 * a collapsed schoolbook product. That path has two shapes and both must be
 * right:
 *
 *   hi == 0  the strength-reduced-division shape (`x /u C` is
 *            `mulhi(x, M) >> s`), one cross term. This is the common case and
 *            the one a division-heavy workload hammers.
 *   hi != 0  a genuine 128-bit multiplier, two cross terms. The second term
 *            consumes lhs.lo, which `mulq` has already overwritten in %rax,
 *            so the low half must be parked before the multiply. Getting that
 *            wrong corrupts only the high 64 bits of the product, which no
 *            division test can see because division keeps only mulhi.
 *
 * Expected values come from a limb-wise reference built from 32-bit pieces,
 * so the oracle shares no codegen with the sequence under test.
 */
#include <stdio.h>
#include <stdlib.h>

typedef unsigned __int128 u128;

/* 128x128 -> low 128 bits, using only 32-bit limbs. Independent of the
 * compiler's i128 multiply: only 32x32->64 products and 64-bit adds. */
static void ref_mul128(const unsigned a[4], const unsigned b[4], unsigned out[4])
{
	unsigned long long acc;
	unsigned carry;
	int i, j;

	for (i = 0; i < 4; i++)
		out[i] = 0;
	for (i = 0; i < 4; i++) {
		carry = 0;
		for (j = 0; i + j < 4; j++) {
			acc = (unsigned long long)out[i + j] +
			      (unsigned long long)a[i] * b[j] + carry;
			out[i + j] = (unsigned)acc;
			carry = (unsigned)(acc >> 32);
		}
	}
}

static void to_limbs(u128 v, unsigned out[4])
{
	int i;

	for (i = 0; i < 4; i++) {
		out[i] = (unsigned)(v & 0xffffffffu);
		v >>= 32;
	}
}

static u128 from_limbs(const unsigned in[4])
{
	u128 v = 0;
	int i;

	for (i = 3; i >= 0; i--)
		v = (v << 32) | in[i];
	return v;
}

#define MULCASE(name, const_expr)                                            \
	__attribute__((noinline)) static u128 name(u128 x)                    \
	{                                                                    \
		return x * (const_expr);                                     \
	}

/* hi == 0: the magic-division shape. */
MULCASE(mul_c1, ((u128)1))
MULCASE(mul_c2, ((u128)2))
MULCASE(mul_c7, ((u128)7))
MULCASE(mul_c1000, ((u128)1000))
MULCASE(mul_c32b, ((u128)0xffffffffu))
MULCASE(mul_c64a, ((u128)0x1234567890abcdefULL))
MULCASE(mul_c64b, ((u128)0xffffffffffffffffULL))
MULCASE(mul_c63, ((u128)0x7fffffffffffffffULL))

/* hi != 0: genuine 128-bit multipliers, two cross terms. */
MULCASE(mul_2p64, (((u128)1 << 64)))
MULCASE(mul_2p64_p1, ((((u128)1 << 64) | 1)))
MULCASE(mul_magic, ((((u128)0xfedcba9876543210ULL << 64) | 0x123456789abcdef0ULL)))
MULCASE(mul_allones, (~(u128)0))
MULCASE(mul_hi_lo, ((((u128)0xffffffffffffffffULL << 64) | 1)))

static u128 (*const cases[])(u128) = {
	mul_c1, mul_c2, mul_c7, mul_c1000, mul_c32b, mul_c64a, mul_c64b,
	mul_c63, mul_2p64, mul_2p64_p1, mul_magic, mul_allones, mul_hi_lo,
};

static const u128 consts[] = {
	(u128)1,
	(u128)2,
	(u128)7,
	(u128)1000,
	(u128)0xffffffffu,
	(u128)0x1234567890abcdefULL,
	(u128)0xffffffffffffffffULL,
	(u128)0x7fffffffffffffffULL,
	((u128)1 << 64),
	(((u128)1 << 64) | 1),
	(((u128)0xfedcba9876543210ULL << 64) | 0x123456789abcdef0ULL),
	~(u128)0,
	(((u128)0xffffffffffffffffULL << 64) | 1),
};

/* Left operands chosen to hit the carries and the high half: zero, the small
 * values, the 64-bit boundary on both sides, and all-ones. */
static const u128 lhs[] = {
	(u128)0,
	(u128)1,
	(u128)2,
	(u128)0x7fffffffu,
	(u128)0x80000000u,
	(u128)0xffffffffu,
	(u128)0x100000000ULL,
	(u128)0x7fffffffffffffffULL,
	(u128)0x8000000000000000ULL,
	(u128)0xffffffffffffffffULL,
	((u128)1 << 64),
	(((u128)1 << 64) | 0xdeadbeefULL),
	(((u128)0x7fffffffffffffffULL << 64) | 0xffffffffffffffffULL),
	~(u128)0,
};

int main(void)
{
	unsigned al[4], bl[4], rl[4];
	unsigned long long fails = 0;
	size_t c, l;

	for (c = 0; c < sizeof(cases) / sizeof(cases[0]); c++) {
		for (l = 0; l < sizeof(lhs) / sizeof(lhs[0]); l++) {
			u128 got = cases[c](lhs[l]);

			to_limbs(lhs[l], al);
			to_limbs(consts[c], bl);
			ref_mul128(al, bl, rl);
			if (got != from_limbs(rl)) {
				unsigned gl[4];

				to_limbs(got, gl);
				printf("FAIL const[%zu] lhs[%zu]: got %08x%08x%08x%08x "
				       "want %08x%08x%08x%08x\n",
				       c, l, gl[3], gl[2], gl[1], gl[0], rl[3],
				       rl[2], rl[1], rl[0]);
				fails++;
			}
		}
	}

	printf("%s (%llu failure%s of %zu cases)\n", fails ? "FAILED" : "ALL-OK",
	       fails, fails == 1 ? "" : "s",
	       (sizeof(cases) / sizeof(cases[0])) * (sizeof(lhs) / sizeof(lhs[0])));
	return fails ? 1 : 0;
}
