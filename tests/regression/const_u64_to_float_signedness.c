/* IrConst::I64 carries both signed and unsigned 64-bit bit patterns. Constant
 * int-to-float casts must obtain signedness from the C expression type, so
 * ~0ULL converts as UINT64_MAX rather than -1. */
#include <stdlib.h>

__attribute__((noinline))
static float runtime_u64_to_float(unsigned long long value)
{
    return value;
}

int main(void)
{
    unsigned long long value = ~0ULL;
    if ((float)~0ULL != runtime_u64_to_float(value))
        abort();
    if ((double)~0ULL != (double)value)
        abort();
    return 0;
}
