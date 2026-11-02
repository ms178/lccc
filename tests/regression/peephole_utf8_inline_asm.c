/*
 * MS-09: inline-assembly templates are text, not narrow-string byte carriers.
 *
 * The source marker contains 2-, 3-, and 4-byte UTF-8 code points.  The
 * executable check below proves that operand substitution and assembly still
 * work; scripts/check_inline_asm_utf8.py additionally requires the exact
 * marker bytes between #APP and #NO_APP in the generated assembly.
 */

static long add_one_with_utf8_comment(long x) {
    long y = x;
    __asm__ volatile(
        "# MS09 UTF-8: café € 🦀\n\t"
        /* The UTF-8 sequences below are deliberately split across adjacent
         * C string literals. parse_asm_string must concatenate raw carrier
         * bytes before decoding them as output text. */
        "# MS09 split UTF-8: caf\303" "\251 \342\202" "\254 \360\237\246" "\200\n\t"
        "addq $1, %0"
        : "+r"(y)
        :
        : "cc");
    return y;
}

int main(void) {
    return add_one_with_utf8_comment(41) != 42;
}
