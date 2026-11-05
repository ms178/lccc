/* Caller half of the by-reference aggregate interop fixture; see
 * psabi_byref_interop.h.  Self-checking: exit status 0 is success, otherwise
 * the number of the first failing check. */
#include "psabi_byref_interop.h"

static volatile int zero;

#define CHECK(id, cond)                                                                        \
    do {                                                                                       \
        if (!(cond))                                                                           \
            return id;                                                                         \
    } while (0)

static long big_hash(const struct Big *b)
{
    long h = 0;
    for (int i = 0; i < 5; i++)
        h = h * 7 + b->a[i];
    return h;
}

static int big_is(const struct Big *b, long a0, long a1, long a2, long a3, long a4)
{
    return b->a[0] == a0 && b->a[1] == a1 && b->a[2] == a2 && b->a[3] == a3 && b->a[4] == a4;
}

/* The value bytes of a long double: x87 extended precision pads 10 bytes. */
#if __LDBL_MANT_DIG__ == 64
#define LD_DATA 10
#else
#define LD_DATA ((int)sizeof(long double))
#endif

static int check_big(void)
{
    struct Big x = {{1 + zero, 2, 3, 4, 5}};
    const long hx = big_hash(&x);
    const long hneg = -1 * (7 * 7 * 7 * 7 + 7 * 7 * 7 + 7 * 7 + 7 + 1);

    CHECK(1, big_mod(x) == hx + hneg);
    CHECK(2, big_is(&x, 1, 2, 3, 4, 5));
    /* The same object twice: the callee's write to p must not reach q. */
    CHECK(3, big_two(x, x) == hx * 3 + 1000);
    CHECK(4, big_is(&x, 1, 2, 3, 4, 5));
    /* An object the callee also reaches through a global. */
    struct Big g0 = gbig;
    CHECK(5, big_glob(gbig) == big_hash(&g0));
    CHECK(6, gbig.a[0] == 500 && gbig.a[2] == 600 && gbig.a[1] == 12);
    gbig = g0;
    /* The result buffer and the argument are the same object. */
    x = big_rev(x);
    CHECK(7, big_is(&x, 10, 8, 6, 4, 2));
    x = big_rev(x);
    CHECK(8, big_is(&x, 4, 8, 12, 16, 20));
    /* Eight general registers taken: the pointer goes to the stack. */
    struct Big y = {{5 + zero, 4, 3, 2, 1}};
    CHECK(9, big_after8(1, 2, 3, 4, 5, 6, 7, 8, y, 9) == big_hash(&y) + 36 + 9000 + 77);
    CHECK(10, big_is(&y, 5, 4, 3, 2, 1));
    return 0;
}

static int check_mix(void)
{
    struct Mix m = {2.5 + zero, {10, 20, 30}};
    struct Mix n = {4.0, {1, 2, 3}};
    CHECK(20, mix_mod(1.0, m, m) == 7 + 10 + 30 * 5 + 2 + 1);
    CHECK(21, m.d == 2.5 && m.l[0] == 10 && m.l[1] == 20 && m.l[2] == 30);
    CHECK(22, mix_mod(0.0, n, m) == 10 + 1 + 150 + 2);
    CHECK(23, n.d == 4.0 && n.l[2] == 3);
    return 0;
}

/* 1.5 + -2.25i, stored byte by byte: converting a double constant could
 * need a libgcc helper at -O0. */
static void cld_set(_Complex long double *z)
{
    unsigned char *p = (unsigned char *)z;
    const int n = (int)sizeof(long double);
    for (int i = 0; i < 2 * n; i++)
        p[i] = 0;
#if __LDBL_MANT_DIG__ == 64
    p[7] = 0xc0, p[8] = 0xff, p[9] = 0x3f;
    p[n + 7] = 0x90, p[n + 8] = 0x00, p[n + 9] = 0xc0;
#elif __LDBL_MANT_DIG__ == 113
    p[13] = 0x80, p[14] = 0xff, p[15] = 0x3f;
    p[n + 13] = 0x20, p[n + 14] = 0x00, p[n + 15] = 0xc0;
#else
    p[6] = 0xf8, p[7] = 0x3f;
    p[n + 6] = 0x02, p[n + 7] = 0xc0;
#endif
}

static int check_cld(void)
{
    _Complex long double z;
    unsigned char *p = (unsigned char *)&z;
    cld_set(&z);
    unsigned char saved[sizeof z];
    long h = 0;
    for (int i = 0; i < (int)sizeof z; i++)
        saved[i] = p[i];
    for (int i = 0; i < (int)sizeof z; i++) {
        /* Only the value bytes of each half are guaranteed to arrive. */
        int in_value = i % (int)sizeof(long double) < LD_DATA;
        h = h * 3 + (in_value ? p[i] : 0);
    }
    long r = cld_mod(z, z);
    if (LD_DATA == (int)sizeof(long double))
        CHECK(30, r == h);
    for (int i = 0; i < (int)sizeof z; i++)
        CHECK(31, p[i] == saved[i]);
    return 0;
}

static int check_va(void)
{
    struct Big x = {{3 + zero, 1, 4, 1, 5}};
    const long hx = big_hash(&x);
    /* The same object twice through `...`: each va_arg reads its own copy. */
    CHECK(40, big_va(2, x, x) == (hx + 4444) * 11 + hx + 4444);
    CHECK(41, big_is(&x, 3, 1, 4, 1, 5));
    return 0;
}

int main(void)
{
    int r = check_big();
    if (!r)
        r = check_mix();
    if (!r)
        r = check_cld();
    if (!r)
        r = check_va();
    return r;
}
