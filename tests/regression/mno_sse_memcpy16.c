/* 16-byte memcpy must stay GPR under `-mno-sse -march=native`.
 * Kernel decompressor pairs those flags; a `movdqu` is #UD (CR4.OSFXSR=0). */
#include <stdio.h>
#include <stdint.h>
#include <string.h>

__attribute__((noinline)) void copy16(void *d, const void *s)
{
	__builtin_memcpy(d, s, 16);
}

int main(void)
{
	uint8_t src[16], dst[16];
	for (int i = 0; i < 16; i++)
		src[i] = (uint8_t)(0xA0 + i);
	memset(dst, 0x5A, 16);
	copy16(dst, src);
	if (memcmp(dst, src, 16) != 0) {
		printf("FAIL\n");
		return 1;
	}
	printf("OK mno_sse_memcpy16\n");
	return 0;
}
