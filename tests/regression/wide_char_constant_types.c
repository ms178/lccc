/* Typed wide and unicode character constants (C11 6.4.4.4p10-11), default
 * mode: L'x' is wchar_t (int), u'x' is char16_t (unsigned short) and U'x' is
 * char32_t (unsigned int). The types are checked with _Generic, and the
 * unsigned element type is observed through a comparison. GCC is the oracle. */
#include <stddef.h>
#include <stdio.h>

#define SAME(e, T) _Generic((e), T: 1, default: 0)

int main(void)
{
	if (!SAME(L'a', wchar_t))
		return 1;
	if (!SAME(L'a', int))
		return 2;
	if (!SAME(u'a', unsigned short))
		return 3;
	if (!SAME(U'a', unsigned int))
		return 4;
	/* U'a' is unsigned, so U'a' - 'b' wraps and is not negative. */
	if ((U'a' - 'b') < 0)
		return 5;
	/* u'a' promotes to int, so the same difference is negative. */
	if (!(u'a' - 'b' < 0))
		return 6;
	if (u'\uFFFF' != 65535)
		return 7;
	if (U'\U0001F600' != 0x1F600u)
		return 8;
	if (L'\x41' != 'A')
		return 9;
	printf("%d %d %d\n", (int)sizeof(L'a'), (int)sizeof(u'a'), (int)sizeof(U'a'));
	printf("%u %u %d\n", (unsigned)u'\uFFFF', (unsigned)U'\U0001F600', (int)L'\x41');
	return 0;
}
