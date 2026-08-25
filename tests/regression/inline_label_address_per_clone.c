/* LabelAddr is a local CFG edge-like reference and must be BlockId-remapped by
 * the inliner. Two clones of one inline body require distinct label values. */
#include <stdlib.h>

static void *first;
static void *second;

static inline void remember(void **slot, int enabled)
{
    if (enabled) {
    here:
        *slot = &&here;
    }
}

__attribute__((noinline)) static void one(int enabled) { remember(&first, enabled); }
__attribute__((noinline)) static void two(int enabled) { remember(&second, enabled); }

int main(void)
{
    one(1);
    two(1);
    if (!first || !second || first == second)
        abort();
    return 0;
}
