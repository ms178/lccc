#include "i386_mixed12_shared.h"

/* The callee half, and the only definition: what `use` does with its parameter is
 * fixed by the psABI's classification of the type, not by whichever compiler
 * happens to be looking. */
unsigned long long use(struct P p)
{
    return p.w + (unsigned long long)(int)p.f;
}
