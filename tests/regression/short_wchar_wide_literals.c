/* -fshort-wchar: `L"..."` is a 16-bit unsigned wide string (GCC parity).
 *
 * WHY THIS TEST EXISTS
 * --------------------
 * arch/x86/mm/fault.c and arch/x86/kernel/setup.c build with -fshort-wchar
 * (the kernel forces it so `wchar_t`-in-source matches the 16-bit CHAR16 the
 * EFI runtime uses). include/linux/efi.h relies on that:
 *
 *     status = get_var(L"SecureBoot", &efi_secure_boot_guid, NULL, &size, data);
 *
 * where get_var takes `efi_char16_t *` = `unsigned short *`. If the literal
 * stays `int[]`, the call is a type error and the kernel does not build.
 * Measured GCC 14.2 semantics this test pins:
 *
 *   gcc -fshort-wchar -dM -E -   ->  __WCHAR_TYPE__  short unsigned int
 *                                    __SIZEOF_WCHAR_T__ 2
 *                                    __WCHAR_MAX__ 0xffff
 *                                    __WCHAR_MIN__ 0
 *                                    __WCHAR_WIDTH__ 16
 *   gcc -fshort-wchar -c         ->  unsigned short g[] = L"\u00e9\u20ac\U0001F600ab";
 *                                    .data = e900ac20 3dd800de 61006200 0000
 *   gcc (default)                ->  same literal rejected for `unsigned short[]`,
 *                                    accepted for `int[]` with 4-byte code units.
 *
 * The bytes above are the whole point: a non-BMP character is a UTF-16
 * SURROGATE PAIR (d83d de00) under -fshort-wchar, and a single 32-bit code
 * point (00f60100) by default. `u"..."` already encodes exactly like that, which
 * is why -fshort-wchar retargets the wide-literal AST node onto that path.
 *
 * Hermetic on purpose: the test never includes a header, because which
 * <stddef.h> wins would otherwise decide what `wchar_t` is, and the property
 * under test is the COMPILER's, not a header's.
 */

extern int printf(const char *, ...);

/* The macro spellings themselves are part of the contract: kernel code and
 * ICU-style feature tests branch on them. */
#if __SIZEOF_WCHAR_T__ != 2
#error "__SIZEOF_WCHAR_T__ must be 2 under -fshort-wchar"
#endif
#if __WCHAR_WIDTH__ != 16
#error "__WCHAR_WIDTH__ must be 16 under -fshort-wchar"
#endif
#if __WCHAR_MAX__ != 0xffff
#error "__WCHAR_MAX__ must be 0xffff under -fshort-wchar"
#endif
#if __WCHAR_MIN__ != 0
#error "__WCHAR_MIN__ must be 0 under -fshort-wchar"
#endif
/* `unsigned short` must be the type, and it must be spelled so that a
 * declaration `__WCHAR_TYPE__ v;` parses under both compilers. */
#ifndef __WCHAR_TYPE__
#error "__WCHAR_TYPE__ must be predefined"
#endif

/* The kernel's EFI call shape: a wide literal must convert to the 16-bit
 * character pointer the parameter expects, with no cast in the source. */
typedef unsigned short efi_char16_t;
static int take16(const efi_char16_t *s)
{
    return s[0] + s[1];
}
static int take_ushort(const unsigned short *s)
{
    return (int)s[0];
}

/* UTF-16 code units of a wide literal, indexed through the literal's own type:
 * this only compiles if `L"..."` decays to a 2-byte element pointer. */
static int check_code_units(void)
{
    static const efi_char16_t *w = L"\u00e9\u20ac\U0001F600ab";

    if (sizeof(L"ab") != 6) /* 2 code units + NUL, 2 bytes each */
        return 1;
    if (w[0] != 0x00e9 || w[1] != 0x20ac)
        return 2;
    if (w[2] != 0xd83d || w[3] != 0xde00) /* U+1F600 surrogate pair */
        return 3;
    if (w[4] != 'a' || w[5] != 'b' || w[6] != 0)
        return 4;
    return 0;
}

int main(void)
{
    int rc = check_code_units();

    if (sizeof(__WCHAR_TYPE__) != 2)
        rc = rc ? rc : 5;
    if (sizeof(L"") != 2) /* just the terminator */
        rc = rc ? rc : 6;

    /* Both parameter shapes must accept the literal uncast (fault.c/setup.c
     * pass it to `efi_char16_t *`; efi.h's get_var passes it straight on). */
    if (take16(L"SecureBoot") == 0)
        rc = rc ? rc : 7;
    if (take_ushort(L"a") != 'a')
        rc = rc ? rc : 8;

    printf("short-wchar: sizeof=%zu code-units-ok=%d take16=%d rc=%d\n",
           sizeof(L"ab"), rc == 0, take16(L"SecureBoot"), rc);
    return rc;
}
