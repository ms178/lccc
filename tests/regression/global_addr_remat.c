/* Executable-data addresses may be rebuilt at pointer-arithmetic uses instead
 * of occupying a register/slot for their whole lifetime. Full PIC and weak
 * externs retain GOT semantics. */
typedef unsigned char u8;

u8 ra01_bytes[256];
unsigned ra01_index = 13;
unsigned ra01_scalar = 7;
extern int ra01_missing __attribute__((weak));

__attribute__((noinline))
unsigned global_addr_probe(unsigned rounds)
{
    unsigned sum = 0;
    for (unsigned i = 0; i < rounds; ++i) {
        unsigned at = (ra01_index + i) & 255;
        u8 *p = ra01_bytes + at;
        *p = (u8)(*p + (u8)(ra01_scalar + i));
        sum += *p;
    }
    return sum;
}

__attribute__((noinline))
unsigned *global_addr_escape(void)
{
    return &ra01_scalar;
}

int main(void)
{
    unsigned a = global_addr_probe(17);
    unsigned b = global_addr_probe(17);
    if (a != 255 || b != 510)
        return 1;
    if (global_addr_escape() != &ra01_scalar || *global_addr_escape() != 7)
        return 2;
    if (&ra01_missing != 0)
        return 3;
    return 0;
}
