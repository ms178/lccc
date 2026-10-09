/* -fshort-wchar: wchar_t is unsigned short and L"..." is UTF-16 (kernel EFI
 * get_var(L"SecureBoot", ...) takes efi_char16_t *). GCC is the oracle. */
#include <stdio.h>
#include <stddef.h>
typedef unsigned short char16;
static int take(const char16 *s) { return s[0] + s[1] * 1000 + (int)sizeof(*s) * 100000; }
int main(void) {
    const wchar_t *w = L"Hi";
    printf("%d %d %d %d\n", (int)sizeof(wchar_t), (int)sizeof(L"ab"), (int)__WCHAR_MAX__, (int)__SIZEOF_WCHAR_T__);
    printf("%d %d\n", (int)w[0], (int)w[1]);
    printf("%d\n", take(L"Hi"));
    return 0;
}
