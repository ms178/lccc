/* _mm_stream_si64 on i686 is two 32-bit MOVNTI stores (64-bit MOVNTI only
 * exists in 64-bit mode).  The constant form used to store the low half and
 * a zero high word, truncating negative and >= 2^32 constants.  GCC offers
 * no 64-bit non-temporal store in 32-bit mode, so this is lccc-only
 * (LCCC_NO_COMPARE in the .env); the self-check is the oracle. */
#include <emmintrin.h>
#include <stdio.h>

static int fail;
static void chk(long long got, long long want, const char *what)
{
	if (got != want) {
		printf("FAIL %s: %llx != %llx\n", what, (unsigned long long)got,
		       (unsigned long long)want);
		fail = 1;
	}
}

__attribute__((noinline)) static void put(long long *p, long long v)
{
	_mm_stream_si64(p, v);
}

int main(void)
{
	long long slot[6] = {0, 0, 0, 0, 0, 0};
	_mm_stream_si64(&slot[0], -1LL);
	_mm_stream_si64(&slot[1], 0x100000001LL);
	_mm_stream_si64(&slot[2], (long long)0x8000000000000000ULL);
	_mm_stream_si64(&slot[3], -2LL);
	put(&slot[4], 0x123456789abcdef0LL);
	put(&slot[5], -0x76543210LL);
	_mm_sfence();
	chk(slot[0], -1LL, "const -1");
	chk(slot[1], 0x100000001LL, "const 2^32+1");
	chk(slot[2], (long long)0x8000000000000000ULL, "const INT64_MIN");
	chk(slot[3], -2LL, "const -2");
	chk(slot[4], 0x123456789abcdef0LL, "variable");
	chk(slot[5], -0x76543210LL, "variable negative");
	if (!fail)
		printf("movnti64 i686 ok\n");
	return fail;
}
