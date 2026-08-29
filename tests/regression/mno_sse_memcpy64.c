/* 64-byte memcpy under `-mno-sse -march=haswell` must not use ymm/xmm.
 * The 64-byte YMM fast path keys off avx2_enabled; sticky -mno-sse must
 * keep that path closed (CR4.OSFXSR=0 in the kernel decompressor). */
#include <stdio.h>
#include <stdint.h>
#include <string.h>

__attribute__((noinline)) void copy64(void *d, const void *s)
{
	__builtin_memcpy(d, s, 64);
}

int main(void)
{
	uint8_t src[64], dst[64];
	for (int i = 0; i < 64; i++)
		src[i] = (uint8_t)(0x40 + i);
	memset(dst, 0xA5, 64);
	copy64(dst, src);
	if (memcmp(dst, src, 64) != 0) {
		printf("FAIL\n");
		return 1;
	}
	printf("OK mno_sse_memcpy64\n");
	return 0;
}
