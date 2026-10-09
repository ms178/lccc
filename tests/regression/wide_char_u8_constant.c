/* C23 u8'x' has type char8_t, which is unsigned char, and its value is
 * truncated to 8 bits. GCC does not accept u8 character constants in C17
 * mode, so this is an lccc-conformance test (see the .txt marker). */
#define SAME(e, T) _Generic((e), T: 1, default: 0)

int main(void)
{
	if (!SAME(u8'a', unsigned char))
		return 1;
	if (sizeof(u8'a') != 1)
		return 2;
	if (u8'\xff' != 255)
		return 3;
	if (!(u8'a' - 'b' < 0))
		return 4;
	return 0;
}
