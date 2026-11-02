/* MS-09 Compiler Explorer companion: byte-exact UTF-8 inline-asm text. */

__attribute__((noinline))
long ms09_utf8_asm_text(long value) {
    __asm__ volatile(
        "# MS09 CE UTF-8: café € 🦀\n\t"
        "addq $1, %0"
        : "+r"(value)
        :
        : "cc");
    return value;
}
