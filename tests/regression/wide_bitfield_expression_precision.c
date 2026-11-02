/* GNU extended bit-field types retain their declared precision through
 * arithmetic. A 40-bit field wraps shifts/subtraction at bit 40, not bit 64. */
#include <stdlib.h>

struct wide { unsigned long long value : 40; };
static struct wide item;

int main(void)
{
    item.value = 0x100;
    if ((item.value << 32) != 0)
        abort();

    item.value = 2;
    if (((unsigned long long)(item.value - 8)) + 8 != 0x10000000002ULL)
        abort();

    item.value = 0x0100000001ULL;
    if ((item.value << 8) + (item.value >> 32) != 0x101)
        abort();
    return 0;
}
