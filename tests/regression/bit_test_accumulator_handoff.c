/* A canonicalized BitTest with a physical result home must still consume an
 * adjacent logical producer assigned only to the accumulator location. */
__attribute__((noinline))
unsigned long probe_bit_acc(unsigned long x, unsigned long y)
{
    unsigned long value = x & y;
    return (value >> 63) & 1;
}

int main(void)
{
    int rc = 0;
    rc |= probe_bit_acc(1UL << 63, ~0UL) != 1;
    rc |= probe_bit_acc(7, ~0UL) != 0;
    rc |= probe_bit_acc(~0UL, 0) != 0;
    return rc;
}
