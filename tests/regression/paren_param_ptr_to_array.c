/* A parenthesized parameter declarator with several stars and an array
 * suffix, `const char (**names)[N]`, is a pointer to a pointer to an array.
 * lccc dropped the outer pointer level from the parameter type, so passing
 * `&names` (a `char (**)[N]`) was rejected as `char (*)[N]`.
 * linux-6.18.55 net/ethtool/privflags.c ethnl_get_priv_flags_info().
 * GCC is the oracle. Output checked against GCC:
 *   8 8 32
 *   8 8 32
 *   1 0
 */
#include <stdio.h>

#define ETH_GSTRING_LEN 32

struct flags_info {
	unsigned int count;
	const char (*names)[ETH_GSTRING_LEN];
};

static int get_info(unsigned int *count, const char (**names)[ETH_GSTRING_LEN])
{
	static const char table[2][ETH_GSTRING_LEN] = { "rx_dma", "tx_dma" };

	*count = 2;
	if (names)
		*names = table;
	return 0;
}

static int fill(struct flags_info *fi)
{
	return get_info(&fi->count, &fi->names);
}

static int two_level(const char (**pp)[ETH_GSTRING_LEN])
{
	return (int)sizeof(pp) + (int)sizeof(*pp) == 16 ? 1 : 0;
}

int main(void)
{
	struct flags_info fi = { 0, NULL };
	const char (*names)[ETH_GSTRING_LEN] = NULL;
	const char (**pp)[ETH_GSTRING_LEN] = &names;
	unsigned int count = 0;

	get_info(&count, &names);
	printf("%zu %zu %zu\n", sizeof(pp), sizeof(*pp), sizeof(**pp));
	fill(&fi);
	printf("%zu %zu %zu\n", sizeof(&fi.names), sizeof(*&fi.names), sizeof(*fi.names));
	printf("%d %d\n", two_level(pp), (int)(names == NULL));
	return 0;
}
