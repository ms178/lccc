/* A non-packed enum is compatible with its implementation-defined integer
 * type (C11 6.7.2.2p4): `unsigned int` when no enumerator is negative, `int`
 * otherwise. So `enum pg *` converts to `unsigned int *` (linux-6.18.55
 * arch/x86/mm/pat/set_memory.c: lookup_address(addr, &level) with
 * `enum pg_level level`). GCC is the oracle. */
#include <stdio.h>

enum pg_level { PG_NONE, PG_4K, PG_2M, PG_1G };
enum signed_level { SL_NEG = -1, SL_ZERO = 0, SL_ONE = 1 };

static unsigned int lookup_address(unsigned long addr, unsigned int *level)
{
	*level = (unsigned int)(addr >> 21) & 3u;
	return (unsigned int)addr & 0xfffu;
}

static int signed_lookup(int *level)
{
	*level = -7;
	return *level;
}

int main(void)
{
	enum pg_level level = PG_NONE;
	enum signed_level sl = SL_ZERO;
	unsigned int off;

	off = lookup_address(0x200000ul + 0x123ul, &level);
	printf("%u %u\n", off, (unsigned int)level);	/* 291 2 */
	printf("%d\n", signed_lookup((int *)&sl));	/* -7 */
	printf("%d\n", (int)sl);			/* -7 */
	return 0;
}
