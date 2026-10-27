/* _mm_stream_pi (SSE1, as in GCC) is an 8-byte non-temporal store of an
 * __m64.  It used to be a plain C struct copy with no permission check, so
 * it compiled under -mno-sse where GCC refuses it.  It is now lowered at the
 * call site: MOVNTI from a GPR where SSE2 is usable (one 64-bit store on
 * x86-64, two 32-bit stores on i686), else a plain 8-byte store.  MMX state
 * is never touched.  Siblings: _m32 (-m32 -msse2) and _sse1 (-m32 SSE1-only).
 * Every value pattern, both spellings, a streamed loop and the untouched
 * neighbours are compared against GCC 16.2. */
#include <xmmintrin.h>
#include <stdio.h>
#include <string.h>

static __m64 m64(unsigned long long v)
{
	__m64 m;
	memcpy(&m, &v, sizeof m);
	return m;
}

static unsigned long long u64(const void *p)
{
	unsigned long long v;
	memcpy(&v, p, sizeof v);
	return v;
}

__attribute__((noinline)) static void put(__m64 *p, __m64 a)
{
	_mm_stream_pi(p, a);
}

__attribute__((noinline)) static void put_raw(unsigned long long *p, unsigned long long v)
{
	__builtin_ia32_movntq(p, v);
}

__attribute__((noinline)) static void stream_xor(__m64 *dst, const unsigned long long *src, int n,
						 unsigned long long k)
{
	for (int i = 0; i < n; i++)
		_mm_stream_pi(dst + i, m64(src[i] ^ k));
}

static const unsigned long long patterns[] = {
	0, 1, 0xffffffffull, 0x100000000ull, 0x8000000000000000ull, 0xffffffffffffffffull,
	0x7fffffff80000000ull, 0x0123456789abcdefull, 0xfedcba9876543210ull, 0x00000000deadbeefull,
};

int main(void)
{
	enum { N = sizeof patterns / sizeof patterns[0] };
	_Alignas(16) unsigned long long buf[N + 2];
	_Alignas(16) __m64 out[N + 2];
	unsigned long long h = 1469598103934665603ull;
	int fail = 0;

	for (int i = 0; i < N; i++) {
		memset(buf, 0xa5, sizeof buf);
		put((__m64 *)&buf[1], m64(patterns[i]));
		put_raw(&buf[0], ~patterns[i]);
		_mm_sfence();
		if (buf[1] != patterns[i] || buf[0] != ~patterns[i] || buf[2] != 0xa5a5a5a5a5a5a5a5ull) {
			printf("FAIL pattern %d\n", i);
			fail = 1;
		}
		for (int j = 0; j < 3; j++)
			h = (h ^ buf[j]) * 1099511628211ull;
	}
	/* Constant operands take the immediate paths. */
	put((__m64 *)&buf[0], m64(0x8000000000000001ull));
	put_raw(&buf[1], 0xffffffff00000000ull);
	_mm_sfence();
	h = (h ^ buf[0]) * 1099511628211ull;
	h = (h ^ buf[1]) * 1099511628211ull;

	memset(out, 0x5a, sizeof out);
	stream_xor(out + 1, patterns, N, 0x0f0f0f0f0f0f0f0full);
	_mm_sfence();
	for (int i = 0; i < N + 2; i++) {
		unsigned long long v = u64(&out[i]);
		unsigned long long want =
			i == 0 || i == N + 1 ? 0x5a5a5a5a5a5a5a5aull : patterns[i - 1] ^ 0x0f0f0f0f0f0f0f0full;
		if (v != want) {
			printf("FAIL loop %d\n", i);
			fail = 1;
		}
		h = (h ^ v) * 1099511628211ull;
	}
	printf("stream_pi %s %016llx\n", fail ? "FAIL" : "ok", h);
	return fail;
}
