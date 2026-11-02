/* Narrow character escapes: a universal character name names a CHARACTER and
 * is encoded in the execution character set (UTF-8 on this target), while
 * \xHH and \NNN name a BYTE.
 *
 * Original defect: lccc chose between the two rules by magnitude ("more than
 * 0xFF means UTF-8"), which is wrong for exactly U+0080..U+00FF. `"\u00e9"
 * produced the single byte E9 instead of the two bytes C3 A9, so a program
 * that compared an escaped string against a UTF-8 literal from elsewhere
 * disagreed with GCC. Measured reference (gcc-16.2, x86-64-linux-gnu):
 *   "\u00e9"       -> C3 A9          (2 bytes)
 *   "\xe9"         -> E9             (1 byte)
 *   "\351"         -> E9             (1 byte)
 *   "\U0001F600"   -> F0 9F 98 80    (4 bytes)
 *   "\u0041"       -> 41             (1 byte)
 *   '\u00e9'       -> 0xC3A9         (multi-character constant, type int)
 * Multi-byte characters keep their magnitude, so the fix is decided by the
 * escape LETTER (u/U) and not by the value.
 */
#include <stdio.h>
#include <string.h>

int main(void) {
    const char *ucn = "\u00e9";
    const char *hex = "\xe9";
    const char *oct = "\351";
    const char *emoji = "\U0001F600";
    const char *basic = "\u0041";
    int fails = 0;

    if (strlen(ucn) != 2 || (unsigned char)ucn[0] != 0xC3 || (unsigned char)ucn[1] != 0xA9) {
        printf("FAIL ucn bytes %02x %02x len %zu\n",
               (unsigned char)ucn[0], (unsigned char)ucn[1], strlen(ucn));
        fails++;
    }
    if (strlen(hex) != 1 || (unsigned char)hex[0] != 0xE9) {
        printf("FAIL hex bytes %02x len %zu\n", (unsigned char)hex[0], strlen(hex));
        fails++;
    }
    if (strlen(oct) != 1 || (unsigned char)oct[0] != 0xE9) {
        printf("FAIL oct bytes %02x len %zu\n", (unsigned char)oct[0], strlen(oct));
        fails++;
    }
    if (strlen(emoji) != 4 || (unsigned char)emoji[0] != 0xF0 || (unsigned char)emoji[3] != 0x80) {
        printf("FAIL emoji len %zu first %02x\n", strlen(emoji), (unsigned char)emoji[0]);
        fails++;
    }
    if (strlen(basic) != 1 || basic[0] != 'A') {
        printf("FAIL basic ucn len %zu\n", strlen(basic));
        fails++;
    }
    if (sizeof("\u00e9") != 3 || sizeof("\xe9") != 2 || sizeof("\U0001F600") != 5) {
        printf("FAIL sizeof %zu %zu %zu\n",
               sizeof("\u00e9"), sizeof("\xe9"), sizeof("\U0001F600"));
        fails++;
    }
    if ('\u00e9' != 0xC3A9) {
        printf("FAIL char literal %#x\n", (unsigned)'\u00e9');
        fails++;
    }
    /* The two spellings must stay distinct: one is a character, one a byte. */
    if (strcmp(ucn, hex) == 0) {
        printf("FAIL ucn and hex escaped to the same string\n");
        fails++;
    }
    if (fails == 0) {
        printf("PASS ucn_escape_bytes\n");
    }
    return fails != 0;
}
