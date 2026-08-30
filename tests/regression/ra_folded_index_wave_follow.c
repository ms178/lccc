/*
 * RA soundness: folded-address consumers vs. coalesce webs, wave seeds and
 * follow hints (vsprintf.c boot blockers, kernel 6.18.47).
 *
 * Three independent shapes, each self-checked against a reference:
 *
 *  1. number()-style digit loop: `tmp[i++] = digits[num % base]` where the
 *     store's GEP folds into SIB addressing and the index is the peeled
 *     zext of the multi-def I32 latch `i`. The operand must stay live to
 *     the store: the digit's def may not land in the index register
 *     between the GEP and the access (the pre-fix kernel wrote
 *     tmp[digit]).
 *
 *  2. time_str()-style call staging: two consecutive conversion calls
 *     whose argument temporaries (arg0 pointer bump, arg2 widened load)
 *     interleave at the staging point. Seeded wave spans carry the last
 *     live point; a follow hint may not share the register with a seeded
 *     span that starts strictly inside the follower.
 *
 *  3. put_dec()-style pointer latch: a pointer bump chain where a later
 *     value follows an earlier producer homed in a register whose seeded
 *     occupancy continues past an allowed die-at-birth touch.
 */
#include <stdio.h>
#include <string.h>

static const char hex_asc[16] = "0123456789abcdef";
static char tmp[24];

/* Shape 1: digit emission with post-increment store through a folded GEP. */
__attribute__((noinline))
static char *emit_digits(char *out, unsigned long long num, int base)
{
	int i = 0;
	const char *digits = hex_asc;

	do {
		tmp[i++] = digits[num % (unsigned long long)base];
		num /= (unsigned long long)base;
	} while (num);
	while (i > 0)
		*out++ = tmp[--i];
	return out;
}

static char *ref_digits(char *out, unsigned long long num, int base)
{
	char r[24];
	int i = 0;
	do {
		r[i++] = hex_asc[num % (unsigned long long)base];
		num /= (unsigned long long)base;
	} while (num);
	while (i > 0)
		*out++ = r[--i];
	return out;
}

/* Shape 2: two staged calls with interleaved arg temporaries. */
struct printf_spec {
	unsigned int flags;
	unsigned int base;
	int field_width;
	int precision;
};

static const struct printf_spec dec_spec = { 0, 10, 0, 2 };
static const struct printf_spec hex_spec = { 0, 16, 0, 4 };

__attribute__((noinline))
static char *stage_pair(char *out, unsigned long long v)
{
	char *p, *q;

	p = emit_digits(out, v, dec_spec.base);
	*p++ = ':';
	q = emit_digits(p, v, hex_spec.base);
	*q++ = ':';
	return q;
}

static char *ref_pair(char *out, unsigned long long v)
{
	char *p = ref_digits(out, v, 10);
	*p++ = ':';
	p = ref_digits(p, v, 16);
	*p++ = ':';
	return p;
}

/* Shape 3: pointer latch with bump chain feeding later staged uses. */
__attribute__((noinline))
static int latch_chain(char *buf, int n)
{
	char *s = buf;
	int total = 0;

	for (int k = 0; k < n; k++) {
		char *t = s + 1;
		char *u = t + 1;
		*s = 'a' + (k % 26);
		*t = 'a' + ((k + 1) % 26);
		*u = 0;
		total += (int)(*s + *t);
		s = u;
	}
	return total;
}

int main(void)
{
	int fail = 0;
	unsigned long long cases[] = {
		0x0ULL, 0x9ULL, 0xffULL, 0x100ULL, 0xdeadbeefULL,
		0xffffffffULL, 0x123456789abcdef0ULL, 0x8000000000000000ULL,
	};
	int bases[] = { 8, 10, 16 };

	for (unsigned c = 0; c < sizeof(cases) / sizeof(cases[0]); c++) {
		for (unsigned b = 0; b < sizeof(bases) / sizeof(bases[0]); b++) {
			char x[128] = { 0 }, y[128] = { 0 };
			char *xe = emit_digits(x, cases[c], bases[b]);
			char *ye = ref_digits(y, cases[c], bases[b]);
			*xe = 0;
			*ye = 0;
			if (strcmp(x, y) != 0) {
				printf("FAIL digits v=%llu base=%d got=%s want=%s\n",
				       cases[c], bases[b], x, y);
				fail = 1;
			}
		}
		char a[192] = { 0 }, b2[192] = { 0 };
		stage_pair(a, cases[c]);
		ref_pair(b2, cases[c]);
		if (strcmp(a, b2) != 0) {
			printf("FAIL pair v=%llu got=%s want=%s\n", cases[c], a, b2);
			fail = 1;
		}
	}

	char buf[256];
	int got = latch_chain(buf, 40);
	int total = 0;
	{
		char *s = buf;
		for (int k = 0; k < 40; k++) {
			total += (int)('a' + (k % 26)) + (int)('a' + ((k + 1) % 26));
			s += 2;
		}
	}
	if (got != total) {
		printf("FAIL latch got=%d want=%d\n", got, total);
		fail = 1;
	}

	if (!fail)
		puts("OK ra folded-index/wave/follow soundness");
	return fail;
}
