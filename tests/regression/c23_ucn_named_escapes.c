// Named universal character escapes: `\N{NAME}` must denote the character with
// that Unicode name, and a narrow literal must encode it as UTF-8 exactly as
// `\uXXXX` does. The spelling is C++23's; GCC 16.2, Clang 23.1.4 and lccc all
// accept it in C (GCC says so under -pedantic).
//
// Every name below is spelled out twice -- once as the escape, once as the
// expected UTF-8 bytes -- so a wrong lookup shows up as a byte mismatch rather
// than as a silently different string. The names cover both algorithmic
// families (`CJK UNIFIED IDEOGRAPH-4E00`, `HANGUL SYLLABLE GA`) and the
// front-coded table, and the expectations are the gcc-16.2 ones.
//
// Names added after GCC 16.2's table was frozen (`JURCHEN CHARACTER-18E00`
// from Unicode 16, `SMALL SEAL CHARACTER-3D000` from Unicode 17) are covered by
// the unit tests in src/frontend/lexer/ucn_name.rs instead: this file has to be
// valid for the oracle too.
#include <stddef.h>
#include <stdio.h>
#include <string.h>

static int failures;

static void check(const char *what, const char *got, const char *want, size_t want_len) {
    if (memcmp(got, want, want_len) != 0 || got[want_len] != '\0') {
        printf("FAIL %s: expected %zu bytes\n", what, want_len);
        for (size_t i = 0; i < want_len; ++i) {
            printf("  want %02x got %02x\n", (unsigned char)want[i], (unsigned char)got[i]);
        }
        ++failures;
    }
}

#define CHECK(name, literal, want, want_len) check(name, "" literal, want, want_len)

// The same characters through the other literal kinds: a code point, not UTF-8.
#define CHECK_UNITS(name, u16, u32, wide, cp)                                  \
    do {                                                                       \
        const unsigned short *s16 = u16;                                       \
        const unsigned int *s32 = u32;                                         \
        const wchar_t *sw = wide;                                              \
        if (s16[0] != (cp) || s16[1] != 0 || s32[0] != (cp) || s32[1] != 0 ||   \
            sw[0] != (cp) || sw[1] != 0) {                                     \
            printf("FAIL %s: wide/UTF-16/UTF-32 unit mismatch\n", name);        \
            ++failures;                                                        \
        }                                                                      \
    } while (0)

int main(void) {
    CHECK("LATIN SMALL LETTER E WITH ACUTE", "\N{LATIN SMALL LETTER E WITH ACUTE}", "\xc3\xa9", 2);
    CHECK("SPACE", "\N{SPACE}", "\x20", 1);
    CHECK("DIGIT ONE", "\N{DIGIT ONE}", "\x31", 1);
    CHECK("NO-BREAK SPACE", "\N{NO-BREAK SPACE}", "\xc2\xa0", 2);
    CHECK("GREEK SMALL LETTER ALPHA", "\N{GREEK SMALL LETTER ALPHA}", "\xce\xb1", 2);
    CHECK("CYRILLIC CAPITAL LETTER A", "\N{CYRILLIC CAPITAL LETTER A}", "\xd0\x90", 2);
    CHECK("ARABIC LETTER ALEF", "\N{ARABIC LETTER ALEF}", "\xd8\xa7", 2);
    CHECK("DEVANAGARI LETTER KA", "\N{DEVANAGARI LETTER KA}", "\xe0\xa4\x95", 3);
    CHECK("COMBINING ACUTE ACCENT", "\N{COMBINING ACUTE ACCENT}", "\xcc\x81", 2);
    CHECK("ZERO WIDTH JOINER", "\N{ZERO WIDTH JOINER}", "\xe2\x80\x8d", 3);
    CHECK("SNOWMAN", "\N{SNOWMAN}", "\xe2\x98\x83", 3);
    CHECK("MUSICAL SYMBOL G CLEF", "\N{MUSICAL SYMBOL G CLEF}", "\xf0\x9d\x84\x9e", 4);
    CHECK("GRINNING FACE", "\N{GRINNING FACE}", "\xf0\x9f\x98\x80", 4);
    CHECK("CJK UNIFIED IDEOGRAPH-4E00", "\N{CJK UNIFIED IDEOGRAPH-4E00}", "\xe4\xb8\x80", 3);
    CHECK("CJK UNIFIED IDEOGRAPH-20000", "\N{CJK UNIFIED IDEOGRAPH-20000}", "\xf0\xa0\x80\x80", 4);
    CHECK("CJK COMPATIBILITY IDEOGRAPH-F900", "\N{CJK COMPATIBILITY IDEOGRAPH-F900}", "\xef\xa4\x80", 3);
    CHECK("TANGUT IDEOGRAPH-17000", "\N{TANGUT IDEOGRAPH-17000}", "\xf0\x97\x80\x80", 4);
    CHECK("EGYPTIAN HIEROGLYPH-13460", "\N{EGYPTIAN HIEROGLYPH-13460}", "\xf0\x93\x91\xa0", 4);
    CHECK("KHITAN SMALL SCRIPT CHARACTER-18B00", "\N{KHITAN SMALL SCRIPT CHARACTER-18B00}", "\xf0\x98\xac\x80", 4);
    CHECK("NUSHU CHARACTER-1B170", "\N{NUSHU CHARACTER-1B170}", "\xf0\x9b\x85\xb0", 4);
    CHECK("HANGUL SYLLABLE GA", "\N{HANGUL SYLLABLE GA}", "\xea\xb0\x80", 3);
    CHECK("HANGUL SYLLABLE GAG", "\N{HANGUL SYLLABLE GAG}", "\xea\xb0\x81", 3);
    CHECK("HANGUL SYLLABLE A", "\N{HANGUL SYLLABLE A}", "\xec\x95\x84", 3);
    CHECK("HANGUL SYLLABLE HIH", "\N{HANGUL SYLLABLE HIH}", "\xed\x9e\xa3", 3);
    CHECK("ARABIC LIGATURE UIGHUR KIRGHIZ YEH WITH HAMZA ABOVE WITH ALEF MAKSURA ISOLATED FORM", "\N{ARABIC LIGATURE UIGHUR KIRGHIZ YEH WITH HAMZA ABOVE WITH ALEF MAKSURA ISOLATED FORM}", "\xef\xaf\xb9", 3);

    // A character constant reads a named escape exactly like `\uXXXX`: the
    // multi-character constant of the UTF-8 bytes, high byte first.
    if ((int)'\N{LATIN SMALL LETTER E WITH ACUTE}' != 0xC3A9) {
        printf("FAIL character constant\n");
        ++failures;
    }
    if ((int)'\N{SNOWMAN}' != 0xE29883) {
        printf("FAIL three-byte character constant\n");
        ++failures;
    }

    // The other literal kinds hold code points: one unit in UTF-16 while the
    // code point is in the BMP, and a surrogate pair above it.
    CHECK_UNITS("SNOWMAN", u"\N{SNOWMAN}", U"\N{SNOWMAN}", L"\N{SNOWMAN}", 0x2603);
    CHECK_UNITS("CJK UNIFIED IDEOGRAPH-4E00", u"\N{CJK UNIFIED IDEOGRAPH-4E00}",
                U"\N{CJK UNIFIED IDEOGRAPH-4E00}", L"\N{CJK UNIFIED IDEOGRAPH-4E00}", 0x4E00);
    CHECK_UNITS("HANGUL SYLLABLE HIH", u"\N{HANGUL SYLLABLE HIH}",
                U"\N{HANGUL SYLLABLE HIH}", L"\N{HANGUL SYLLABLE HIH}", 0xD7A3);
    {
        const unsigned short *s16 = u"\N{GRINNING FACE}";
        const unsigned int *s32 = U"\N{GRINNING FACE}";
        const wchar_t *sw = L"\N{GRINNING FACE}";
        if (s16[0] != 0xD83D || s16[1] != 0xDE00 || s16[2] != 0 ||
            s32[0] != 0x1F600 || s32[1] != 0 || sw[0] != 0x1F600 || sw[1] != 0) {
            printf("FAIL GRINNING FACE: unit mismatch\n");
            ++failures;
        }
    }

    // Adjacent named escapes and neighbouring literal text must not merge.
    if (strcmp("x\N{LATIN SMALL LETTER A}\N{SNOWMAN}y", "xa\xe2\x98\x83y") != 0) {
        printf("FAIL adjacency\n");
        ++failures;
    }

    printf("%s c23_ucn_named_escapes\n", failures ? "FAIL" : "PASS");
    return failures != 0;
}
