/* Large integer literals must still parse (saturated) and compile. */
int main(void) {
    /* 2^64 is too large for u64 — lexer saturates; value is implementation-defined
       at this stage but must not crash the compiler. */
    unsigned long long x = 0xFFFFFFFFFFFFFFFFULL;
    if (x != 0xFFFFFFFFFFFFFFFFULL) return 1;
    unsigned long long y = 18446744073709551615ULL;
    if (y != x) return 2;
    return 0;
}
