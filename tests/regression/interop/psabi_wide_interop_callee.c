/* Callee half of the complex / 2×XLEN / RV64-flattening interop fixture; see
 * psabi_wide_interop.h. */
#include <stdarg.h>
#include "psabi_wide_interop.h"

/* The value bytes of a long double: x87 extended precision pads 10 bytes. */
#if __LDBL_MANT_DIG__ == 64
#define LD_DATA 10
#else
#define LD_DATA ((int)sizeof(long double))
#endif

static unsigned long long bytes_hash(const void *p, int n)
{
    const unsigned char *b = (const unsigned char *)p;
    unsigned long long h = 0;
    for (int i = 0; i < n; i++)
        h = h * 31 + b[i];
    return h;
}

static unsigned long long ld_hash(const long double *x) { return bytes_hash(x, LD_DATA); }

static unsigned long long cld_hash(const _Complex long double *z)
{
    const long double *parts = (const long double *)z;
    return ld_hash(&parts[0]) * 7 + ld_hash(&parts[1]);
}

double cd_after7(double x0, double x1, double x2, double x3, double x4, double x5, double x6,
                 _Complex double z, double tail)
{
    return x0 + x1 + x2 + x3 + x4 + x5 + x6 + 128 * __real__ z + 256 * __imag__ z + 1024 * tail;
}

double cf_after7(double x0, double x1, double x2, double x3, double x4, double x5, double x6,
                 _Complex float z, double tail)
{
    return x0 + x1 + x2 + x3 + x4 + x5 + x6 + 128 * __real__ z + 256 * __imag__ z + 1024 * tail;
}

double cd_after8(double x0, double x1, double x2, double x3, double x4, double x5, double x6,
                 double x7, _Complex double z, double tail)
{
    return x0 + x1 + x2 + x3 + x4 + x5 + x6 + x7 + 128 * __real__ z + 256 * __imag__ z +
           1024 * tail;
}

unsigned long long cld_after7(double x0, double x1, double x2, double x3, double x4, double x5,
                              double x6, _Complex long double z, double tail)
{
    return (unsigned long long)(x0 + x1 + x2 + x3 + x4 + x5 + x6 + 1024 * tail) +
           cld_hash(&z);
}

unsigned long long cld_bits(_Complex long double z, int k) { return cld_hash(&z) + k; }

double cplx_mix(float f, _Complex float a, double d, _Complex double b, long k)
{
    return f + 2 * __real__ a + 4 * __imag__ a + 8 * d + 16 * __real__ b + 32 * __imag__ b + 64 * k;
}

/* n doubles, then `_Complex double`, `_Complex float`, a double and a
 * `_Complex long double` (bits). */
double va_cplx(int n, ...)
{
    va_list ap;
    va_start(ap, n);
    double s = 0;
    for (int i = 0; i < n; i++)
        s += va_arg(ap, double);
    _Complex double z = va_arg(ap, _Complex double);
    _Complex float f = va_arg(ap, _Complex float);
    double t = va_arg(ap, double);
    _Complex long double l = va_arg(ap, _Complex long double);
    va_end(ap);
    return s + 16 * __real__ z + 32 * __imag__ z + 64 * __real__ f + 128 * __imag__ f +
           256 * t + (double)(cld_hash(&l) & 0xffff) * 1024;
}

_Complex double ret_cd(double re, double im)
{
    _Complex double z = re;
    __imag__ z = im;
    return z;
}

_Complex float ret_cf(float re, float im)
{
    _Complex float z = re;
    __imag__ z = im;
    return z;
}

unsigned long long ld_a7(long a0, long a1, long a2, long a3, long a4, long a5, long a6,
                         long double x, long after)
{
    return (unsigned long long)(a0 + a1 + a2 + a3 + a4 + a5 + a6) + ld_hash(&x) +
           (unsigned long long)after * 1000003;
}

__int128 i128_a7(long a0, long a1, long a2, long a3, long a4, long a5, long a6, __int128 x,
                 long after)
{
    return x ^ ((__int128)(a0 + a1 + a2 + a3 + a4 + a5 + a6) << 64) ^ after;
}

long s2_a7(long a0, long a1, long a2, long a3, long a4, long a5, long a6, struct S2 s,
           long after)
{
    return a0 + a1 + a2 + a3 + a4 + a5 + a6 + s.a * 100 + s.b * 10000 + after * 1000000;
}

/* n longs, then a long double (bits) and a long. */
unsigned long long va_ld(int n, ...)
{
    va_list ap;
    va_start(ap, n);
    long s = 0;
    for (int i = 0; i < n; i++)
        s += va_arg(ap, long);
    long double x = va_arg(ap, long double);
    long after = va_arg(ap, long);
    va_end(ap);
    return (unsigned long long)s + ld_hash(&x) + (unsigned long long)after * 1000003;
}

/* n longs, then an __int128 and a long. */
__int128 va_i128(int n, ...)
{
    va_list ap;
    va_start(ap, n);
    long s = 0;
    for (int i = 0; i < n; i++)
        s += va_arg(ap, long);
    __int128 x = va_arg(ap, __int128);
    long after = va_arg(ap, long);
    va_end(ap);
    return x ^ ((__int128)s << 64) ^ after;
}

double sum_dp(struct DP s) { return s.d + (s.p ? 1 : 0); }
double sum_fb(struct FB s) { return s.f + 2 * s.b; }
double sum_cz(struct CZ s) { return __real__ s.z + 2 * __imag__ s.z; }
double sum_cdm(struct CD s) { return __real__ s.z + 2 * __imag__ s.z; }
double sum_fz(struct FZ s) { return s.a + 2 * __real__ s.z + 4 * __imag__ s.z; }

struct DP ret_dp(double d, void *p) { return (struct DP){d, p}; }

struct FB ret_fb(float f, int b)
{
    struct FB r;
    r.f = f;
    r.b = b;
    return r;
}

struct CZ ret_cz(float re, float im)
{
    struct CZ r;
    __real__ r.z = re;
    __imag__ r.z = im;
    return r;
}

struct CD ret_cdm(double re, double im)
{
    struct CD r;
    __real__ r.z = re;
    __imag__ r.z = im;
    return r;
}

struct FZ ret_fz(float a, float re, float im)
{
    struct FZ r;
    r.a = a;
    __real__ r.z = re;
    __imag__ r.z = im;
    return r;
}

double sum_wd(struct WD s) { return s.a + 2 * s.b; }
double sum_wo(struct WO s) { return 3 * s.d; }
double sum_wm(struct WM s) { return s.f + 2 * (double)s.x; }
double sum_wi(struct WI s) { return (double)s.x + 2 * s.d; }

double wd_after7(double x0, double x1, double x2, double x3, double x4, double x5, double x6,
                 struct WD s, double tail)
{
    return x0 + x1 + x2 + x3 + x4 + x5 + x6 + 10 * s.a + 100 * s.b + 1000 * tail;
}

double wm_after8(long a0, long a1, long a2, long a3, long a4, long a5, long a6, long a7,
                 struct WM s, double tail)
{
    return (double)(a0 + a1 + a2 + a3 + a4 + a5 + a6 + a7) + 10 * s.f + 100 * (double)s.x +
           1000 * tail;
}

/* The parameter is the callee's own object: writing it must not reach the
 * caller's argument. */
double scribble_wd(struct WD s)
{
    double r = s.a + s.b;
    s.a = -1;
    s.b = -2;
    return r + 0 * (s.a + s.b);
}

/* Whole-record copies into the result: a callee that homed only 16 bytes of
 * a 32- or 64-byte result would overwrite its own frame. */
static struct WD wd_template = {7, 8};
static struct WO wo_template = {9};
static struct WM wm_template = {1, 2};
static struct WI wi_template = {3, 4};

struct WD ret_wd(double a, double b)
{
    struct WD r = wd_template;
    r.a = a;
    r.b = b;
    return r;
}

struct WO ret_wo(double d)
{
    struct WO r = wo_template;
    r.d = d;
    return r;
}

struct WM ret_wm(float f, long x)
{
    struct WM r = wm_template;
    r.f = f;
    r.x = x;
    return r;
}

struct WI ret_wi(long x, double d)
{
    struct WI r = wi_template;
    r.x = x;
    r.d = d;
    return r;
}
