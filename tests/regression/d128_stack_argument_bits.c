/* Stack-overflow _Decimal128 arguments carry a 16-byte BID VALUE, not an
 * aggregate address. Exhaust the SysV XMM argument file on x86-64 and check
 * both limbs bit for bit; the i386 twin exercises the stack-only emitter. */
union d128_bits {
    _Decimal128 value;
    unsigned long long word[2];
};

__attribute__((noinline))
int d128_stack_probe(
    double d0, double d1, double d2, double d3,
    double d4, double d5, double d6, double d7,
    _Decimal128 x, unsigned long long lo, unsigned long long hi, unsigned tag)
{
    union d128_bits u = { x };
    (void)d0; (void)d1; (void)d2; (void)d3;
    (void)d4; (void)d5; (void)d6; (void)d7;
    return u.word[0] != lo || u.word[1] != hi || tag != 0x5a5a5a5au;
}

#define PROBE(x, lo, hi) \
    d128_stack_probe(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, \
                     x, lo, hi, 0x5a5a5a5au)

int main(void)
{
    if (PROBE(9.0DD, 0x5a, 0x303e000000000000ULL)) return 1;
    if (PROBE(0, 0, 0x3040000000000000ULL)) return 2;
    if (PROBE(-0.0DD, 0, 0xb03e000000000000ULL)) return 3;

    /* Preserve a finite BID image with nonzero bits in both limbs. */
    volatile union d128_bits variable = {
        .word = { 0x0123456789abcdefULL, 0x303e000000000000ULL }
    };
    if (PROBE(variable.value, 0x0123456789abcdefULL, 0x303e000000000000ULL))
        return 4;

    /* Decimal zeros and coefficients retain their conversion-specific BID
       quantum images; integer and literal spellings are not interchangeable. */
    if (PROBE(9, 9, 0x3040000000000000ULL)) return 5;
    if (PROBE(0.0DD, 0, 0x303e000000000000ULL)) return 6;
    return 0;
}
