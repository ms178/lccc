/* glibc cabsf128 lowers the imaginary zero in hypotf128(real, 0) as an
 * integer-zero IR constant.  The SysV binary128 argument materializer must
 * accept that canonical zero and place 128 zero bits in its XMM register. */
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

int main(void)
{
    return 0;
}
