/* _mm_stream_ps is SSE1 MOVNTPS.  On an SSE1-only target (-mno-sse2 -msse)
 * it used to be a plain C store; it is now the non-temporal store itself,
 * fed by a MOVUPS load (MOVDQU/MOVUPD are SSE2).  The values and the
 * untouched neighbours are checked on both ABIs; -m32 is the sibling file. */
#include <xmmintrin.h>
#include <stdio.h>
#include <string.h>

/* No double anywhere: x86-64 -mno-sse2 has no double lowering (fails closed). */
static unsigned bits(float f)
{
	unsigned u;
	memcpy(&u, &f, sizeof u);
	return u;
}

static int fail;
static void chk(int cond, const char *what)
{
	if (!cond) {
		printf("FAIL %s\n", what);
		fail = 1;
	}
}

__attribute__((noinline)) static void stream_scaled(float *dst, const float *src, int n, float k)
{
	__m128 vk = _mm_set1_ps(k);
	for (int i = 0; i < n; i += 4)
		_mm_stream_ps(dst + i, _mm_mul_ps(_mm_loadu_ps(src + i), vk));
	_mm_sfence();
}

int main(void)
{
	_Alignas(32) float dst[24];
	float src[16];
	for (int i = 0; i < 24; i++)
		dst[i] = -7.0f;
	for (int i = 0; i < 16; i++)
		src[i] = (float)i - 5.5f;
	_mm_stream_ps(dst, _mm_setr_ps(1.5f, -2.25f, 3.0e38f, -0.0f));
	chk(dst[0] == 1.5f && dst[1] == -2.25f && dst[2] == 3.0e38f, "movntps vals");
	chk(bits(dst[3]) == 0x80000000u, "movntps -0.0");
	chk(dst[4] == -7.0f && dst[23] == -7.0f, "movntps neighbours");
	stream_scaled(dst + 4, src, 16, 0.5f);
	for (int i = 0; i < 16; i++)
		chk(dst[4 + i] == src[i] * 0.5f, "stream_scaled");
	chk(dst[20] == -7.0f, "stream_scaled tail");
	unsigned h = 2166136261u;
	for (int i = 0; i < 24; i++)
		h = (h ^ bits(dst[i])) * 16777619u;
	printf("movntps sse1 hash %08x\n", h);
	return fail;
}
