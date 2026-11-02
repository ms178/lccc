#include "i386_mixed12_shared.h"

/* The caller half: poison the bytes around the object, hand it over, and let the
 * answer say whether the callee got the twelve bytes it was promised. */
int main(void)
{
    union {
        struct P p;
        unsigned char raw[16];
    } u;

    for (unsigned i = 0; i < sizeof u.raw; i++)
        u.raw[i] = 0xAA;
    u.p.w = 0x100000001ULL;
    u.p.f = 3.0f;
    return use(u.p) == 0x100000004ULL ? 0 : 1;
}
