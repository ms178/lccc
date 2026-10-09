/* -fshort-wchar: wchar_t is unsigned short, so L'x' has that type and
 * promotes to int in arithmetic. u'x' and U'x' keep their own types. The
 * wchar_t typedef from <stddef.h> must agree with the constant's type.
 * GCC is the oracle. */
#include <stddef.h>
#include <stdio.h>

#define SAME(e, T) _Generic((e), T: 1, default: 0)

int main(void)
{
	if (!SAME(L'a', wchar_t))
		return 1;
	if (!SAME(L'a', unsigned short))
		return 2;
	if (!SAME(u'a', unsigned short))
		return 3;
	if (!SAME(U'a', unsigned int))
		return 4;
	if (sizeof(L'a') != 2)
		return 5;
	if (!(L'a' - 'b' < 0))
		return 6;
	if (L'\xFFFF' != 65535)
		return 7;
	if (L'\x1234' != 0x1234)
		return 8;
	printf("%d %d %d\n", (int)sizeof(wchar_t), (int)sizeof(L'a'), (int)(L'\xFFFF'));
	return 0;
}
