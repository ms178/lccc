/* -fshort-wchar must retype `L"..."` to the 16-bit character type.
 *
 * WHY THIS TEST EXISTS
 * --------------------
 * The kernel builds with -fshort-wchar (`make V=1` shows
 * `-std=gnu18 -fshort-wchar -funsigned-char ...`), because the EFI runtime
 * speaks UTF-16 and include/linux/efi.h passes wide literals straight into
 * 16-bit character pointers:
 *
 *     status = get_var(L"SecureBoot", &efi_secure_boot_guid, NULL, &size, data);
 *
 * where get_var's first parameter is `efi_char16_t *` = `unsigned short *`.
 * Before the fix lccc kept `L"..."` as `int[]` regardless of the flag and
 * rejected the call:
 *
 *     error: incompatible pointer type for argument 1 to 'function pointer':
 *            have 'int *' but expected 'unsigned short *'
 *
 * killing arch/x86/mm/fault.o and arch/x86/kernel/setup.o (include/linux/
 * efi.h:1042 and :1051). Measured GCC 14.2 semantics, which this test pins:
 *
 *   __WCHAR_TYPE__      int                  -> short unsigned int
 *   __SIZEOF_WCHAR_T__  4                    -> 2
 *   __WCHAR_MAX__       0x7fffffff           -> 0xffff
 *   __WCHAR_MIN__       (-__WCHAR_MAX__ - 1) -> 0
 *   sizeof(L"ab")       12                   -> 6
 *   g32[] = L"a\u20ac"  .long 61,20ac,0      -> (rejected: int[] from u16[])
 *   g16[] = L"a\u20ac"  (rejected)           -> .short 61,20ac,0
 *
 * The literal's bytes under -fshort-wchar are UTF-16: a code point above
 * U+FFFF is a surrogate pair, exactly as for u"...". That is why the
 * implementation retargets the wide literal onto the existing char16_t path
 * rather than adding a second 16-bit encoding to keep in step.
 *
 * The test is compiled with -fshort-wchar by its .flags file and must build
 * and run identically under GCC (the differential oracle).
 */
#include <stddef.h>
#include <stdio.h>

typedef unsigned short efi_char16_t;

static int take_efi16(const efi_char16_t *s) /* 'get_var' first parameter */
{
    int n = 0;
    while (s[n])
        n++;
    return n;
}

/* Elements are wchar_t: 2 bytes each, so the literal is 6 bytes wide. */
static unsigned short g16[] = L"ab"; /* len 2 + NUL, 2 bytes per element */
static wchar_t g_wchar[] = L"ab";

/* A code point above U+FFFF needs a surrogate pair in a 16-bit wchar_t. */
static unsigned short g_astral[] = L"\U0001F600";

int main(void)
{
    int fails = 0;
    const char *why = "ok";

    if (sizeof(wchar_t) != 2) { fails++; why = "sizeof(wchar_t) != 2"; }
    if (sizeof(L"ab") != 6) { fails++; why = "sizeof(L\"ab\") != 6"; }
    if (sizeof(g16) != 6) { fails++; why = "sizeof(g16) != 6"; }
    if (sizeof(g_wchar) != 6) { fails++; why = "sizeof(wchar_t[3]) != 6"; }
    if (sizeof(g_astral) != 6) { fails++; why = "astral literal not a surrogate pair"; }
    if (g_astral[0] != 0xD83D || g_astral[1] != 0xDE00) {
        fails++;
        why = "astral code units are not the UTF-16 surrogate pair";
    }
    if (take_efi16(L"SecureBoot") != 10) { fails++; why = "take_efi16(L\"SecureBoot\") != 10"; }

    printf("short_wchar: fails=%d (%s)\n", fails, why);
    return fails != 0;
}
