/* Arithmetic on a >32-bit bit-field must retain the field's 64-bit promoted
 * type. Using I32 for the compound operation discards the high half before
 * reinsertion into the storage unit. */
#include <stdlib.h>

struct wide_fields {
    signed long long pad : 12;
    signed long long value : 52;
};

__attribute__((noinline))
static struct wide_fields scramble(struct wide_fields input)
{
    input.value ^= 0x8765412345678LL;
    return input;
}

int main(void)
{
    struct wide_fields value = {0x123, 0x123456789ABCDLL};
    value = scramble(value);
    if (value.pad != 0x123 || value.value != 0xFFF9551175BDFDB5LL)
        abort();
    return 0;
}
