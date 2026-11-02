/* glibc cabsf128 lowers the imaginary zero in hypotf128(real, 0) as an
 * integer-zero IR constant.  The SysV binary128 argument materializer must
 * accept that canonical zero and place 128 zero bits in its XMM register.
 * A floating -0.0F128, however, is an I128 payload whose sign bit must survive
 * the same call path; it must never match the integer-zero fast path. */
__attribute__((noinline))
_Float128 consume_f128(_Float128 first, _Float128 second)
{
    (void) first;
    return second;
}

_Float128 pass_integer_zero(_Float128 value)
{
    return consume_f128(value, 0);
}

_Float128 pass_negative_zero(_Float128 value)
{
    return consume_f128(value, -0.0F128);
}

union f128_bits {
    _Float128 value;
    unsigned long long word[2];
};

int main(void)
{
    union f128_bits positive = { pass_integer_zero(1.0F128) };
    union f128_bits negative = { pass_negative_zero(1.0F128) };
    return positive.word[0] != 0 || positive.word[1] != 0
        || negative.word[0] != 0
        || negative.word[1] != 0x8000000000000000ULL;
}
