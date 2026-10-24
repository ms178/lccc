/* Stack-overflow _Float128 arguments carry a 16-byte VALUE, not an aggregate
 * address. Eight doubles exhaust the SysV XMM argument file on x86-64; i386
 * already passes the carrier on the stack. Check both limbs bit for bit. */
union f128_bits {
    _Float128 value;
    unsigned long long word[2];
};

__attribute__((noinline))
int f128_stack_probe(
    double d0, double d1, double d2, double d3,
    double d4, double d5, double d6, double d7,
    _Float128 x, unsigned long long lo, unsigned long long hi, unsigned tag)
{
    union f128_bits u = { x };
    (void)d0; (void)d1; (void)d2; (void)d3;
    (void)d4; (void)d5; (void)d6; (void)d7;
    return u.word[0] != lo || u.word[1] != hi || tag != 0x5a5a5a5au;
}

#define PROBE(x, lo, hi) \
    f128_stack_probe(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, \
                     x, lo, hi, 0x5a5a5a5au)

int main(void)
{
    /* binary128 9.0 has a zero low limb: the old aggregate-address path
       materialized that limb in RAX and dereferenced NULL. */
    if (PROBE(9.0F128, 0, 0x4002200000000000ULL)) return 1;
    if (PROBE(0, 0, 0)) return 2;
    if (PROBE(-0.0F128, 0, 0x8000000000000000ULL)) return 3;

    /* A volatile, value-dependent object forces the SSA-slot path and checks
       every bit of a quiet-NaN payload, not merely numeric conversion. */
    volatile union f128_bits variable = {
        .word = { 0x0123456789abcdefULL, 0x7fff800000000000ULL }
    };
    if (PROBE(variable.value, 0x0123456789abcdefULL, 0x7fff800000000000ULL))
        return 4;

    /* A nonzero integer must be converted by lowering, not reinterpreted as
       an I128 object image or rejected by the backend's constant decoder. */
    if (PROBE(9, 0, 0x4002200000000000ULL)) return 5;
    return 0;
}
