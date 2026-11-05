/* Caller half of the complex / 2×XLEN / RV64-flattening interop fixture; see
 * psabi_wide_interop.h.  Self-checking: exit status 0 is success, otherwise
 * the number of the first failing check. */
#include "psabi_wide_interop.h"

static volatile int zero;

#define CHECK(id, cond)                                                                        \
    do {                                                                                       \
        if (!(cond))                                                                           \
            return id;                                                                         \
    } while (0)

/* The value bytes of a long double: x87 extended precision pads 10 bytes. */
#if __LDBL_MANT_DIG__ == 64
#define LD_DATA 10
#else
#define LD_DATA ((int)sizeof(long double))
#endif

static unsigned long long ld_hash(const long double *x)
{
    const unsigned char *b = (const unsigned char *)x;
    unsigned long long h = 0;
    for (int i = 0; i < LD_DATA; i++)
        h = h * 31 + b[i];
    return h;
}

static unsigned long long cld_hash(const _Complex long double *z)
{
    const long double *parts = (const long double *)z;
    return ld_hash(&parts[0]) * 7 + ld_hash(&parts[1]);
}

/* Arbitrary bytes: only moved and compared, never computed with. */
static void ld_fill(long double *p, unsigned seed)
{
    unsigned char *b = (unsigned char *)p;
    for (int i = 0; i < LD_DATA; i++)
        b[i] = (unsigned char)(seed * 13 + i * 7 + zero);
}

static int check_complex(void)
{
    double z = zero;
    _Complex double cd = 0.5 + z;
    __imag__ cd = 0.25 + z;
    _Complex float cf = 0.5f + (float)z;
    __imag__ cf = 0.25f;
    CHECK(1, cd_after7(1, 2, 3, 4, 5, 6, 7 + z, cd, 2.0) == 2204.0);
    CHECK(2, cf_after7(1, 2, 3, 4, 5, 6, 7 + z, cf, 2.0) == 2204.0);
    CHECK(3, cd_after8(1, 2, 3, 4, 5, 6, 7, 8 + z, cd, 2.0) == 2212.0);

    _Complex long double cld;
    ld_fill((long double *)&cld, 3);
    ld_fill((long double *)&cld + 1, 11);
    unsigned long long h = cld_hash(&cld);
    CHECK(4, cld_after7(1, 2, 3, 4, 5, 6, 7 + z, cld, 2.0) == 2076 + h);
    CHECK(5, cld_bits(cld, 5) == h + 5);

    _Complex double b = 3.0 + z;
    __imag__ b = 4.0;
    CHECK(6, cplx_mix(1.5f, cf, 2.0, b, 3) == 387.5);

    _Complex float g = 1.5f + (float)z;
    __imag__ g = 2.5f;
    CHECK(7, va_cplx(7, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0 + z, cd, g, 3.0, cld) ==
                 1228 + (double)(h & 0xffff) * 1024);
    CHECK(8, va_cplx(0, cd, g, 3.0 + z, cld) == 1200 + (double)(h & 0xffff) * 1024);

    _Complex double r = ret_cd(1.5 + z, -2.25);
    CHECK(9, __real__ r == 1.5 && __imag__ r == -2.25);
    _Complex float rf = ret_cf(1.5f + (float)z, -2.25f);
    CHECK(10, __real__ rf == 1.5f && __imag__ rf == -2.25f);
    return 0;
}

static int check_wide(void)
{
    long k = zero;
    long double x;
    ld_fill(&x, 5);
    unsigned long long h = ld_hash(&x);
    CHECK(20, ld_a7(1, 2, 3, 4, 5, 6, 7 + k, x, 9) == 28 + h + 9ull * 1000003);

    __int128 w = ((__int128)0x0123456789abcdefll << 64) | 0xfedcba9876543210ull;
    w += k;
    __int128 want = w ^ ((__int128)28 << 64) ^ 9;
    CHECK(21, i128_a7(1, 2, 3, 4, 5, 6, 7 + k, w, 9) == want);

    struct S2 s = {3 + k, 5};
    CHECK(22, s2_a7(1, 2, 3, 4, 5, 6, 7 + k, s, 9) == 9050328);

    /* Unnamed: 5 longs leave a6/a7 (an even pair), 6 leave a7 (skipped), 7
     * leave nothing. */
    CHECK(23, va_ld(5, 1l, 2l, 3l, 4l, 5l + k, x, 9l) == 15 + h + 9ull * 1000003);
    CHECK(24, va_ld(6, 1l, 2l, 3l, 4l, 5l, 6l + k, x, 9l) == 21 + h + 9ull * 1000003);
    CHECK(25, va_ld(7, 1l, 2l, 3l, 4l, 5l, 6l, 7l + k, x, 9l) == 28 + h + 9ull * 1000003);
    CHECK(26, va_i128(5, 1l, 2l, 3l, 4l, 5l + k, w, 9l) == (w ^ ((__int128)15 << 64) ^ 9));
    CHECK(27, va_i128(6, 1l, 2l, 3l, 4l, 5l, 6l + k, w, 9l) == (w ^ ((__int128)21 << 64) ^ 9));
    CHECK(28, va_i128(7, 1l, 2l, 3l, 4l, 5l, 6l, 7l + k, w, 9l) == want);
    return 0;
}

static int check_flatten(void)
{
    float f = zero;
    void *p = (void *)&zero;
    struct DP dp = {2.5 + f, p};
    CHECK(40, sum_dp(dp) == 3.5);
    struct FB fb = {1.5f + f, -3};
    CHECK(41, sum_fb(fb) == -4.5);
    struct CZ cz;
    __real__ cz.z = 1.5f + f;
    __imag__ cz.z = 2.5f;
    CHECK(42, sum_cz(cz) == 6.5);
    struct CD cdm;
    __real__ cdm.z = 1.5 + f;
    __imag__ cdm.z = 2.5;
    CHECK(43, sum_cdm(cdm) == 6.5);
    struct FZ fz;
    fz.a = 0.5f + f;
    __real__ fz.z = 1.5f;
    __imag__ fz.z = 2.5f;
    CHECK(44, sum_fz(fz) == 13.5);

    struct DP rdp = ret_dp(2.5 + f, p);
    CHECK(50, rdp.d == 2.5 && rdp.p == p);
    struct FB rfb = ret_fb(1.5f + f, -3);
    CHECK(51, rfb.f == 1.5f && rfb.b == -3);
    struct CZ rcz = ret_cz(1.5f + f, 2.5f);
    CHECK(52, __real__ rcz.z == 1.5f && __imag__ rcz.z == 2.5f);
    struct CD rcd = ret_cdm(1.5 + f, 2.5);
    CHECK(53, __real__ rcd.z == 1.5 && __imag__ rcd.z == 2.5);
    struct FZ rfz = ret_fz(0.5f + f, 1.5f, 2.5f);
    CHECK(54, rfz.a == 0.5f && __real__ rfz.z == 1.5f && __imag__ rfz.z == 2.5f);
    return 0;
}

static int check_overaligned(void)
{
    double f = zero;
    struct WD wd = {1.5 + f, 2.5};
    CHECK(60, sum_wd(wd) == 6.5);
    struct WO wo = {1.5 + f};
    CHECK(61, sum_wo(wo) == 4.5);
    struct WM wm = {0.5f + (float)f, -3};
    CHECK(62, sum_wm(wm) == -5.5);
    struct WI wi = {-3, 1.25 + f};
    CHECK(63, sum_wi(wi) == -0.5);
    CHECK(64, wd_after7(1, 2, 3, 4, 5, 6, 7 + f, wd, 0.5) == 28 + 15 + 250 + 500);
    CHECK(65, wm_after8(1, 2, 3, 4, 5, 6, 7, 8, wm, 0.25 + f) == 36 + 5 - 300 + 250);
    CHECK(66, scribble_wd(wd) == 4.0 && wd.a == 1.5 && wd.b == 2.5);

    struct WD rwd = ret_wd(1.5 + f, 2.5);
    CHECK(70, rwd.a == 1.5 && rwd.b == 2.5);
    struct WO rwo = ret_wo(-1.5 + f);
    CHECK(71, rwo.d == -1.5);
    struct WM rwm = ret_wm(0.5f + (float)f, -7);
    CHECK(72, rwm.f == 0.5f && rwm.x == -7);
    struct WI rwi = ret_wi(-9, 2.25 + f);
    CHECK(73, rwi.x == -9 && rwi.d == 2.25);
    return 0;
}

int main(void)
{
    int r = check_complex();
    if (!r)
        r = check_wide();
    if (!r)
        r = check_flatten();
    if (!r)
        r = check_overaligned();
    return r;
}
