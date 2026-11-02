/* Local array initialized from a string literal (C11 6.7.9p14/p15/p21):
 *  - the elements after the terminator are zero, for EVERY element width
 *    (wide `wchar_t w[10] = L"ab";` left w[3..9] uninitialized);
 *  - when the array has room only for the characters, the terminator is
 *    dropped, never stored past the object (the wide and char16_t forms
 *    wrote it one unit beyond `T a[2] = L"ab";`, over the next object);
 *  - a wide/char16_t array MEMBER takes the characters, not the literal's
 *    address (only char members were recognised);
 *  - a short literal in a big array zero-fills the tail with a bounded
 *    sequence (gcc.c-torture/compile/20151204.c: 32751 byte stores);
 *  - u"" literals are UTF-16 (C11 6.4.5p6): a code point above U+FFFF is a
 *    surrogate pair in the contents AND in every size (it was truncated to
 *    its low 16 bits and counted as one unit).
 * Every array is dumped element by element; gcc is the oracle. */
#include <stdio.h>
#include <string.h>
#include <wchar.h>
#include <uchar.h>

__attribute__((noinline)) void dirty(void)
{
    volatile unsigned char junk[4096];
    memset((unsigned char *)junk, 0x5a, sizeof junk);
}

static unsigned long mix(const void *p, size_t n)
{
    const unsigned char *b = p;
    unsigned long h = 1469598103u;
    for (size_t i = 0; i < n; i++)
        h = (h ^ b[i]) * 16777619u;
    return h;
}

__attribute__((noinline)) unsigned long wide_tail(void)
{
    wchar_t w[10] = L"ab";
    char32_t u[7] = U"xyz";
    return mix(w, sizeof w) ^ mix(u, sizeof u);
}

__attribute__((noinline)) unsigned long char16_tail(void)
{
    char16_t c[9] = u"hi";
    char16_t d[5] = {u"q"};
    return mix(c, sizeof c) ^ mix(d, sizeof d);
}

__attribute__((noinline)) unsigned long exact_fit(void)
{
    volatile int before = 0x11111111;
    wchar_t w[2] = L"ab";
    char16_t c[2] = u"cd";
    char s[2] = "ef";
    volatile int after = 0x22222222;
    return mix(w, sizeof w) ^ mix(c, sizeof c) ^ mix(s, sizeof s) ^ (unsigned long)before
           ^ (unsigned long)after;
}

struct members {
    wchar_t w[2];
    int guard1;
    char16_t c[3];
    unsigned short guard2;
    char32_t u[4];
    char16_t braced[4];
    int guard3;
};

__attribute__((noinline)) unsigned long member_init(void)
{
    struct members m = { L"ab", 0x12345678, u"xy", 0xBEEF, U"k", {u"z"}, 0x7654321 };
    return mix(&m, sizeof m);
}

__attribute__((noinline)) unsigned long big_tail(void)
{
    char t[4099] = "A";         /* memset path */
    char odd[29] = "abc";       /* byte head, then 8-byte ladder */
    char mid[130] = "0123456";  /* just above the ladder limit */
    return mix(t, sizeof t) ^ mix(odd, sizeof odd) ^ mix(mid, sizeof mid);
}

static const char16_t g16[] = u"a\U0001F600b";
static char16_t g16_member_holder[2][4] = { u"\U00010348", u"z" };

__attribute__((noinline)) unsigned long utf16_units(void)
{
    char16_t c[] = u"x\U0001F600y";
    char16_t cut[2] = u"a\U0001F600";       /* keeps 'a' and the high surrogate */
    const char16_t *p = u"\U0010FFFF!";
    struct { char16_t m[4]; int guard; } s = { u"\U0001D11E", 0x5a5a };
    const char16_t *lit = (const char16_t[]){ u"\U0001F4A9" };
    unsigned long h = mix(c, sizeof c) ^ mix(cut, sizeof cut) ^ mix(&s, sizeof s)
                      ^ mix(g16, sizeof g16) ^ mix(g16_member_holder, sizeof g16_member_holder);
    h = h * 31 + sizeof c + sizeof g16 + sizeof(u"\U0001F600") + u"\U0001F600"[1];
    for (int i = 0; i < 4; i++)
        h = h * 31 + p[i] + lit[i < 3 ? i : 2];
    return h;
}

int main(void)
{
    dirty();
    printf("%lx\n", wide_tail());
    dirty();
    printf("%lx\n", char16_tail());
    dirty();
    printf("%lx\n", exact_fit());
    dirty();
    printf("%lx\n", member_init());
    dirty();
    printf("%lx\n", big_tail());
    dirty();
    printf("%lx\n", utf16_units());
    return 0;
}
