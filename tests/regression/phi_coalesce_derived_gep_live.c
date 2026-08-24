/* A destructive loop-phi update must not overwrite an old IV while an address
 * derived from it is still waiting to be consumed.  GVN merges the two i+1
 * expressions; x86 then folds the GEP into the Store's SIB operand. */
#include <stdlib.h>

static unsigned char same_index[256];
static unsigned char next_value[256];

int main(void)
{
    for (int i = 0; i < 256; ++i) {
        same_index[i] = (unsigned char)i;
        next_value[i] = (unsigned char)(i + 1);
    }
    for (int i = 0; i < 256; ++i) {
        if (same_index[i] != (unsigned char)i)
            abort();
        if (next_value[i] != (unsigned char)(i + 1))
            abort();
    }
    return 0;
}
