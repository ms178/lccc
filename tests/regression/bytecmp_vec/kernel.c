/*
 * Emission-contract kernel for the byte-compare window phase.
 *
 * Kept separate from the two behavioural drivers so the gate can compile ONE
 * small function and compare exact instruction counts: the drivers contain
 * their own machinery (mmap setup, expectation tables), whose codegen would
 * otherwise sit in the counts the gate asserts on.
 *
 * Both stream types are here because the arm accepts U8 and I8 loads and must
 * treat them identically; the signed pointer forces Slt-bounded IR for the
 * bound test, which is the case that catches a room test built from the wrong
 * comparison op.
 *
 * Referenced by tests/regression/check_bytecmp_vec_codegen.sh.
 */
const unsigned char *bytecmp_unsigned(const unsigned char *p,
                                      const unsigned char *end,
                                      const unsigned char *q)
{
    while (p < end && *p == *q) { p++; q++; }
    return p;
}

const char *bytecmp_signed(const char *p, const char *end, const char *q)
{
    while (p < end && *p == *q) { p++; q++; }
    return p;
}
