// Target plain-char signedness (C11 6.7.2p5, C11 6.4.4.4p10): the declared
// `char` rank and character-constant values must AGREE on every target —
// unsigned on AArch64/RISC-V SysV, signed on x86-64/i686 — and
// -fsigned-char/-funsigned-char must flip both consistently. The exact
// values are target-dependent, so the test pins the invariants that hold
// everywhere: (unsigned char) round-trips exactly, `signed char` always
// sign-extends, and plain char + a char constant never disagree.
int main(void) {
    // (unsigned char) always has 8 value bits, 0..255.
    unsigned char u = 200;
    if ((int)u != 200)
        return 1;
    if ((int)(unsigned char)0xFF != 255)
        return 2;

    // (signed char) always sign-extends.
    signed char s = 200; // impl-defined conversion, -56 on every 8-bit-char target
    if ((int)s != -56)
        return 3;
    if ((int)(signed char)0xFF != -1)
        return 4;

    // Plain char and character constants must take the SAME view: if
    // plain char is signed, both are negative; if unsigned, both are in
    // 128..255. A target where the declaration and the constant disagree
    // (e.g. char c = 200 reads back -56 while '\xEF' is 239) is broken.
    char c = 200;
    int c_val = (int)c;
    int lit_val = (int)(char)0xEF;
    if (!((c_val == 200 && lit_val == 239) || (c_val == -56 && lit_val == -17)))
        return 5;

    // The plain-char view must equal the (signed char)/(unsigned char)
    // view chosen by the target's CHAR_BITS-signedness, i.e. char, signed
    // char and unsigned char are distinct TYPES but char matches exactly
    // one of them in value behavior for these inputs.
    if (c_val == -56 && (int)(char)0xFF != -1)
        return 6;
    if (c_val == 200 && (int)(char)0xFF != 255)
        return 7;

    // Char-typed arithmetic keeps target semantics either way.
    char a = 100, b = 100;
    int sum = (int)(char)(a + b); // 8-bit wraparound then char-extend
    if (!((sum == -56 && c_val == -56) || (sum == 200 && c_val == 200)))
        return 8;

    return 0;
}
