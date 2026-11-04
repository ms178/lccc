/* Callee half of the by-reference aggregate interop fixture; see
 * psabi_byref_interop.h.  Every function writes to its own by-value
 * parameters, as an optimizing GCC or Clang callee may. */
#include <stdarg.h>
#include "psabi_byref_interop.h"

struct Big gbig = {{11, 12, 13, 14, 15}};

static long big_hash(const struct Big *b)
{
    long h = 0;
    for (int i = 0; i < 5; i++)
        h = h * 7 + b->a[i];
    return h;
}

long big_mod(struct Big b)
{
    long h = big_hash(&b);
    for (int i = 0; i < 5; i++)
        b.a[i] = -1;
    return h + big_hash(&b);
}

long big_two(struct Big p, struct Big q)
{
    p.a[0] = 1000;
    p.a[4] = 2000;
    return big_hash(&q) * 3 + p.a[0];
}

long big_glob(struct Big b)
{
    gbig.a[0] = 500;
    gbig.a[2] = 600;
    return big_hash(&b);
}

struct Big big_rev(struct Big b)
{
    struct Big r;
    for (int i = 0; i < 5; i++)
        r.a[i] = b.a[4 - i] * 2;
    return r;
}

long big_after8(long r0, long r1, long r2, long r3, long r4, long r5, long r6, long r7,
                struct Big b, long tail)
{
    long h = big_hash(&b);
    b.a[1] = 77;
    return h + r0 + r1 + r2 + r3 + r4 + r5 + r6 + r7 + tail * 1000 + b.a[1];
}

long mix_mod(double f, struct Mix m, struct Mix n)
{
    m.d += 1.0;
    m.l[2] = 99;
    return (long)(m.d * 2) + m.l[0] + n.l[2] * 5 + (long)n.d + (long)f;
}

long cld_mod(_Complex long double z, _Complex long double w)
{
    unsigned char *p = (unsigned char *)&z;
    const unsigned char *q = (const unsigned char *)&w;
    long h = 0;
    for (int i = 0; i < (int)sizeof z; i++) {
        h = h * 3 + q[i];
        p[i] = 0x5a;
    }
    return h;
}

long big_va(int n, ...)
{
    va_list ap;
    va_start(ap, n);
    long h = 0;
    for (int i = 0; i < n; i++) {
        struct Big b = va_arg(ap, struct Big);
        h = h * 11 + big_hash(&b);
        b.a[3] = 4444;
        h += b.a[3];
    }
    va_end(ap);
    return h;
}
