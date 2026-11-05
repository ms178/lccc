/* Callee half of the floating-point aggregate interop fixture; see
 * psabi_hfa_interop.h. */
#include <stdarg.h>
#include "psabi_hfa_interop.h"

/* Bit patterns of long double / _Float16 values without arithmetic. */
/* The value bytes of a long double: x87 extended precision pads its 10
 * bytes to 16, and a copy through the FPU stack need not keep the padding. */
#if __LDBL_MANT_DIG__ == 64
#define LD_DATA 10
#else
#define LD_DATA ((int)sizeof(long double))
#endif

static unsigned long long ld_bits(const long double *p)
{
    const unsigned char *b = (const unsigned char *)p;
    unsigned long long h = 0;
    for (int i = 0; i < LD_DATA; i++)
        h = h * 31 + b[i];
    return h;
}

static unsigned h_bits(const _Float16 *p)
{
    const unsigned char *b = (const unsigned char *)p;
    return b[0] | (unsigned)b[1] << 8;
}

double sum_f2(struct F2 s) { return s.a + 2 * s.b; }
double sum_f3(struct F3 s) { return s.a + 2 * s.b + 4 * s.c; }
double sum_f4(struct F4 s) { return s.a + 2 * s.b + 4 * s.c + 8 * s.d; }
double sum_d2(struct D2 s) { return s.a + 2 * s.b; }
double sum_d3(struct D3 s) { return s.a + 2 * s.b + 4 * s.c; }
double sum_d4(struct D4 s) { return s.a + 2 * s.b + 4 * s.c + 8 * s.d; }
double sum_nf(struct NF s) { return s.in.a + 2 * s.in.b + 4 * s.c; }
double sum_ad(struct AD s) { return s.v[0] + 2 * s.v[1] + 4 * s.v[2]; }
double sum_ud(union UD s) { return s.v[0] + 2 * s.s.b; }
double sum_zb(struct ZB s) { return s.a + 2 * s.b; }
unsigned sum_h3(struct H3 s) { return h_bits(&s.a) + 3 * h_bits(&s.b) + 9 * h_bits(&s.c); }
unsigned long long bits_ld1(struct LD1 s) { return ld_bits(&s.a); }
unsigned long long bits_ld2(int k, struct LD2 s) { return ld_bits(&s.a) * 7 + ld_bits(&s.b) + k; }
double sum_v4f(v4f v) { return v[0] + 2 * v[1] + 4 * v[2] + 8 * v[3]; }
double sum_v2f(v2f v) { return v[0] + 2 * v[1]; }
long long sum_v4i(v4i v) { return v[0] + 2 * v[1] + 4 * v[2] + 8 * v[3]; }
double sum_v2(struct V2 s) { return sum_v4f(s.x) + 16 * sum_v4f(s.y); }
double sum_vm(struct VM s) { return sum_v2f(s.x) + 4 * sum_v2f(s.y); }
double sum_mx(struct MX s) { return s.a + 2 * s.b; }
double sum_f5(struct F5 s) { return s.a + 2 * s.b + 4 * s.c + 8 * s.d + 16 * s.e; }
double sum_fi(struct FI s) { return s.f + 2 * s.i; }
double sum_id(struct ID s) { return s.i + 2 * s.d; }
double sum_cf(struct CF s) { return s.c + 2 * s.f; }
double sum_ds(struct DS s) { return s.d + 2 * s.s; }

double mix_regs(int i, struct D3 a, float f, struct F4 b, long long l, double d)
{
    return i + 2 * sum_d3(a) + 4 * f + 8 * sum_f4(b) + 16 * l + 32 * d;
}

double nsrn_overflow(double x0, double x1, double x2, double x3, double x4, double x5,
                     struct D3 s, double after, int k)
{
    return x0 + 2 * x1 + 4 * x2 + 8 * x3 + 16 * x4 + 32 * x5 + 64 * sum_d3(s) + 512 * after + k;
}

double nsrn_overflow_f2(double x0, double x1, double x2, double x3, double x4, double x5,
                        double x6, struct F2 s, float g)
{
    return x0 + 2 * x1 + 4 * x2 + 8 * x3 + 16 * x4 + 32 * x5 + 64 * x6 + 128 * sum_f2(s) + 512 * g;
}

double quads_in_regs(struct D4 a, struct D4 b) { return sum_d4(a) + 16 * sum_d4(b); }

double quads_spill(struct D4 a, struct D4 b, struct D4 c, double tail)
{
    return sum_d4(a) + 16 * sum_d4(b) + 256 * sum_d4(c) + 4096 * tail;
}

unsigned long long ld_spill(double x0, double x1, double x2, double x3, double x4, double x5,
                            double x6, struct LD2 s, struct LD1 t)
{
    return (unsigned long long)(x0 + x1 + x2 + x3 + x4 + x5 + x6) + ld_bits(&s.a) * 5 +
           ld_bits(&s.b) * 3 + ld_bits(&t.a);
}

double cplx_then_struct(_Complex double z, struct D2 p, struct FI q)
{
    return __real__ z + 2 * __imag__ z + 4 * sum_d2(p) + 16 * sum_fi(q);
}

double cplx_after7(double x0, double x1, double x2, double x3, double x4, double x5, double x6,
                   _Complex double z, double tail)
{
    return x0 + x1 + x2 + x3 + x4 + x5 + x6 + 128 * __real__ z + 256 * __imag__ z + 1024 * tail;
}

struct F2 ret_f2(float a, float b) { return (struct F2){a, b}; }
struct F3 ret_f3(float a, float b, float c) { return (struct F3){a, b, c}; }
struct F4 ret_f4(float a) { return (struct F4){a, a + 1, a + 2, a + 3}; }
struct D2 ret_d2(double a, double b) { return (struct D2){a, b}; }
struct D3 ret_d3(double a) { return (struct D3){a, a * 2, a * 4}; }
struct D4 ret_d4(double a) { return (struct D4){a, a + 0.5, a + 1, a + 1.5}; }
struct NF ret_nf(float a) { return (struct NF){{a, a + 1}, a + 2}; }
struct AD ret_ad(double a) { return (struct AD){{a, a + 0.25, a + 0.5}}; }
union UD ret_ud(double a) { union UD u; u.v[0] = a; u.v[1] = a + 4; return u; }

struct H3 ret_h3(unsigned short a)
{
    struct H3 h;
    unsigned char *p = (unsigned char *)&h;
    for (int i = 0; i < 3; i++) {
        unsigned short v = a + i;
        p[2 * i] = v & 0xff;
        p[2 * i + 1] = v >> 8;
    }
    return h;
}

struct LD2 ret_ld2(unsigned long long a, unsigned long long b)
{
    struct LD2 r;
    unsigned char *p = (unsigned char *)&r;
    for (int i = 0; i < 16; i++) {
        p[i] = (a >> (i % 8 * 8)) + i;
        p[16 + i] = (b >> (i % 8 * 8)) + i;
    }
    return r;
}

v4f ret_v4f(float a) { return (v4f){a, a + 1, a + 2, a + 3}; }
v2f ret_v2f(float a) { return (v2f){a, a * 2}; }
struct V2 ret_v2(float a) { return (struct V2){ret_v4f(a), ret_v4f(a + 4)}; }
struct VM ret_vm(float a) { return (struct VM){ret_v2f(a), ret_v2f(a + 8)}; }
struct MX ret_mx(double a) { return (struct MX){(float)a, a * 2}; }
struct F5 ret_f5(float a) { return (struct F5){a, a + 1, a + 2, a + 3, a + 4}; }
struct FI ret_fi(float f, int i) { return (struct FI){f, i}; }
struct ID ret_id(int i, double d) { return (struct ID){i, d}; }
struct CF ret_cf(char c, float f) { return (struct CF){c, f}; }
struct DS ret_ds(double d, short s) { return (struct DS){d, s}; }
struct D1 ret_d1(double d) { return (struct D1){d}; }

struct D4 scale_d4(struct D4 s, double k)
{
    s.a *= k, s.b *= k, s.c *= k, s.d *= k;
    return s;
}

struct F3 rotate_f3(struct F3 s) { return (struct F3){s.b, s.c, s.a}; }

double call_maker(struct D3 (*mk)(double), double x)
{
    struct D3 r = mk(x);
    return sum_d3(r);
}

/* Tags: 1 F3, 2 D4, 3 v4f, 4 double, 5 D2, 6 V2, 7 LD1 (bits), 8 F5, 9 MX,
 * 10 H3 (bits), 11 NF. */
static double va_one(int tag, va_list *ap)
{
    switch (tag) {
    case 1: return sum_f3(va_arg(*ap, struct F3));
    case 2: return sum_d4(va_arg(*ap, struct D4));
    case 3: return sum_v4f(va_arg(*ap, v4f));
    case 4: return va_arg(*ap, double);
    case 5: return sum_d2(va_arg(*ap, struct D2));
    case 6: return sum_v2(va_arg(*ap, struct V2));
    case 7: { struct LD1 t = va_arg(*ap, struct LD1); return (double)(ld_bits(&t.a) & 0xffff); }
    case 8: return sum_f5(va_arg(*ap, struct F5));
    case 9: return sum_mx(va_arg(*ap, struct MX));
    case 10: return sum_h3(va_arg(*ap, struct H3));
    case 11: return sum_nf(va_arg(*ap, struct NF));
    }
    return -1e9;
}

/* n tags packed four bits each in the first variadic int. */
double va_hfa(int n, ...)
{
    va_list ap;
    va_start(ap, n);
    unsigned long long tags = va_arg(ap, unsigned long long);
    double s = 0, w = 1;
    for (int i = 0; i < n; i++, w *= 2)
        s += va_one((tags >> (4 * i)) & 15, &ap) * w;
    va_end(ap);
    return s;
}

/* Seven unnamed doubles exhaust all but one SIMD slot; the D2 after them
 * goes to the stack and closes the SIMD file, so the final double is read
 * from the stack too. */
double va_spill(int n, ...)
{
    va_list ap;
    va_start(ap, n);
    double s = 0;
    for (int i = 0; i < n; i++)
        s += va_arg(ap, double);
    struct D2 p = va_arg(ap, struct D2);
    double last = va_arg(ap, double);
    long long k = va_arg(ap, long long);
    va_end(ap);
    return s + 16 * sum_d2(p) + 256 * last + 1024 * k;
}

double va_named_hfa(struct D3 s, ...)
{
    va_list ap;
    va_start(ap, s);
    double a = va_arg(ap, double);
    struct F3 f = va_arg(ap, struct F3);
    double b = va_arg(ap, double);
    va_end(ap);
    return sum_d3(s) + 8 * a + 16 * sum_f3(f) + 256 * b;
}

double va_named_overflow(double x0, double x1, double x2, double x3, double x4, double x5,
                         struct D3 s, ...)
{
    va_list ap;
    va_start(ap, s);
    double a = va_arg(ap, double);
    struct F2 f = va_arg(ap, struct F2);
    long long k = va_arg(ap, long long);
    va_end(ap);
    return x0 + x1 + x2 + x3 + x4 + x5 + 8 * sum_d3(s) + 64 * a + 128 * sum_f2(f) + 1024 * k;
}

static double sum_da32(struct DA32 s) { return s.a + 2 * s.b + 4 * s.c + 8 * s.d; }

/* n doubles, then a struct DA32 and a double. */
double va_da32(int n, ...)
{
    va_list ap;
    va_start(ap, n);
    double s = 0;
    for (int i = 0; i < n; i++)
        s += va_arg(ap, double);
    struct DA32 v = va_arg(ap, struct DA32);
    double tail = va_arg(ap, double);
    va_end(ap);
    return s + 16 * sum_da32(v) + 1024 * tail;
}

double da32_after9(double x0, double x1, double x2, double x3, double x4, double x5, double x6,
                   double x7, double x8, struct DA32 s, double tail)
{
    return x0 + x1 + x2 + x3 + x4 + x5 + x6 + x7 + x8 + 16 * sum_da32(s) + 1024 * tail;
}
