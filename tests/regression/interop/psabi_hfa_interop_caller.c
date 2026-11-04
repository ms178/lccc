/* Caller half of the floating-point aggregate interop fixture; see
 * psabi_hfa_interop.h.  Self-checking: exit status 0 is success, otherwise
 * the number of the first failing check. */
#include "psabi_hfa_interop.h"

static volatile int zero;

#define CHECK(id, cond)                                                                        \
    do {                                                                                       \
        if (!(cond))                                                                           \
            return id;                                                                         \
    } while (0)

static unsigned long long ld_bits(const long double *p)
{
    const unsigned char *b = (const unsigned char *)p;
    unsigned long long h = 0;
    for (int i = 0; i < 16; i++)
        h = h * 31 + b[i];
    return h;
}

static void ld_fill(long double *p, unsigned seed)
{
    unsigned char *b = (unsigned char *)p;
    for (int i = 0; i < 16; i++)
        b[i] = (unsigned char)(seed * 13 + i * 7);
}

static void h_fill(_Float16 *p, unsigned short v)
{
    unsigned char *b = (unsigned char *)p;
    b[0] = v & 0xff;
    b[1] = v >> 8;
}

static unsigned h_bits(const _Float16 *p)
{
    const unsigned char *b = (const unsigned char *)p;
    return b[0] | (unsigned)b[1] << 8;
}

static struct D3 make_d3(double a) { return (struct D3){a, a + 1, a + 2}; }

/* Called through a pointer so the compiler cannot see the definition. */
static double (*volatile f3_sum)(struct F3) = sum_f3;

static int check_args(void)
{
    const float z = (float)zero;
    struct F2 f2 = {1.5f + z, 0.25f};
    struct F3 f3 = {0.5f, 1.0f + z, 2.0f};
    struct F4 f4 = {0.25f, 0.5f, 0.75f + z, 1.0f};
    struct D2 d2 = {3.5 + z, 0.125};
    struct D3 d3 = {1.0, 2.0 + z, 3.0};
    struct D4 d4 = {0.5, 1.5, 2.5 + z, 3.5};
    struct NF nf = {{1.0f, 2.0f + z}, 0.5f};
    struct AD ad = {{0.25, 0.5 + z, 0.75}};
    union UD ud;
    ud.v[0] = 6.0 + z;
    ud.v[1] = 0.5;
    struct ZB zb = {2.0f + z, 0.75f};
    struct H3 h3;
    h_fill(&h3.a, 0x3c00 + zero);
    h_fill(&h3.b, 0x4000);
    h_fill(&h3.c, 0xbc00);
    struct LD1 ld1;
    ld_fill(&ld1.a, 3 + zero);
    struct LD2 ld2;
    ld_fill(&ld2.a, 5);
    ld_fill(&ld2.b, 9 + zero);
    v4f v4 = {1.0f + z, 0.5f, 0.25f, 2.0f};
    v2f v2 = {4.0f, 0.5f + z};
    v4i vi = {1 + zero, -2, 3, 40};
    struct V2 vv = {{1, 2, 3, 4 + z}, {0.5f, 0.25f, 0.125f, 1}};
    struct VM vm = {{1, 2 + z}, {3, 0.5f}};
    struct MX mx = {0.5f + z, 4.25};
    struct F5 f5 = {1, 2, 3 + z, 4, 0.5f};
    struct FI fi = {1.25f + z, -3};
    struct ID id = {7 + zero, 0.75};
    struct CF cf = {'A', 0.5f + z};
    struct DS ds = {2.5 + z, -9};

    CHECK(1, sum_f2(f2) == 2.0);
    CHECK(2, sum_f3(f3) == 10.5);
    CHECK(3, sum_f4(f4) == 12.25);
    CHECK(4, sum_d2(d2) == 3.75);
    CHECK(5, sum_d3(d3) == 17.0);
    CHECK(6, sum_d4(d4) == 41.5);
    CHECK(7, sum_nf(nf) == 7.0);
    CHECK(8, sum_ad(ad) == 4.25);
    CHECK(9, sum_ud(ud) == 7.0);
    CHECK(10, !ZB_INTEROP || sum_zb(zb) == 3.5);
    CHECK(11, sum_h3(h3) == 0x3c00u + 3 * 0x4000u + 9 * 0xbc00u);
    CHECK(12, bits_ld1(ld1) == ld_bits(&ld1.a));
    CHECK(13, bits_ld2(4, ld2) == ld_bits(&ld2.a) * 7 + ld_bits(&ld2.b) + 4);
    CHECK(14, sum_v4f(v4) == 19.0);
    CHECK(15, sum_v2f(v2) == 5.0);
    CHECK(16, sum_v4i(vi) == 1 - 4 + 12 + 320);
    CHECK(17, sum_v2(vv) == 49.0 + 16 * 9.5);
    CHECK(18, sum_vm(vm) == 5.0 + 4 * 4.0);
    CHECK(19, sum_mx(mx) == 9.0);
    CHECK(20, sum_f5(f5) == 1 + 4 + 12 + 32 + 8);
    CHECK(21, sum_fi(fi) == -4.75);
    CHECK(22, sum_id(id) == 8.5);
    CHECK(23, sum_cf(cf) == 66.0);
    CHECK(24, sum_ds(ds) == -15.5);
    CHECK(25, mix_regs(3, d3, 0.5f, f4, -2, 0.25) == 3 + 34 + 2 + 98 - 32 + 8);
    CHECK(26, nsrn_overflow(1, 2, 3, 4, 5, 6 + z, d3, 0.5, 9) ==
                  1 + 4 + 12 + 32 + 80 + 192 + 64 * 17.0 + 256 + 9);
    CHECK(27, nsrn_overflow_f2(1, 2, 3, 4, 5, 6, 7 + z, f2, 0.5f) ==
                  1 + 4 + 12 + 32 + 80 + 192 + 448 + 256 + 256);
    CHECK(28, quads_in_regs(d4, ret_d4(1.0)) == 41.5 + 16 * (1 + 3 + 8 + 20));
    CHECK(29, quads_spill(d4, d4, ret_d4(2.0), 0.5) ==
                  41.5 * 17 + 256 * (2 + 5 + 12 + 28) + 2048);
    CHECK(30, ld_spill(1, 2, 3, 4, 5, 6, 7 + z, ld2, ld1) ==
                  28 + ld_bits(&ld2.a) * 5 + ld_bits(&ld2.b) * 3 + ld_bits(&ld1.a));
    _Complex double c = 1.5 + z;
    __imag__ c = 0.25;
    CHECK(31, cplx_then_struct(c, d2, fi) == 1.5 + 0.5 + 4 * 3.75 + 16 * -4.75);
    CHECK(32, cplx_after7(1, 2, 3, 4, 5, 6, 7 + z, c, 0.5) == 28 + 192 + 64 + 512);
    CHECK(33, f3_sum(f3) == 10.5);
    CHECK(34, call_maker(make_d3, 2.0 + z) == 2 + 6 + 16);
    return 0;
}

static int check_returns(void)
{
    const float z = (float)zero;
    struct F2 f2 = ret_f2(1.5f + z, -2.0f);
    CHECK(40, f2.a == 1.5f && f2.b == -2.0f);
    struct F3 f3 = ret_f3(0.5f, 0.25f + z, 8.0f);
    CHECK(41, f3.a == 0.5f && f3.b == 0.25f && f3.c == 8.0f);
    struct F4 f4 = ret_f4(1.0f + z);
    CHECK(42, f4.a == 1 && f4.b == 2 && f4.c == 3 && f4.d == 4);
    struct D2 d2 = ret_d2(3.0 + z, 0.75);
    CHECK(43, d2.a == 3.0 && d2.b == 0.75);
    struct D3 d3 = ret_d3(0.5 + z);
    CHECK(44, d3.a == 0.5 && d3.b == 1.0 && d3.c == 2.0);
    struct D4 d4 = ret_d4(4.0 + z);
    CHECK(45, d4.a == 4.0 && d4.b == 4.5 && d4.c == 5.0 && d4.d == 5.5);
    struct NF nf = ret_nf(2.0f + z);
    CHECK(46, nf.in.a == 2 && nf.in.b == 3 && nf.c == 4);
    struct AD ad = ret_ad(1.0 + z);
    CHECK(47, ad.v[0] == 1.0 && ad.v[1] == 1.25 && ad.v[2] == 1.5);
    union UD ud = ret_ud(2.0 + z);
    CHECK(48, ud.v[0] == 2.0 && ud.v[1] == 6.0);
    struct H3 h3 = ret_h3(0x3c00 + zero);
    CHECK(49, h_bits(&h3.a) == 0x3c00 && h_bits(&h3.b) == 0x3c01 && h_bits(&h3.c) == 0x3c02);
    struct LD2 ld2 = ret_ld2(0x0102030405060708ull + zero, 0x1112131415161718ull);
    const unsigned char *p = (const unsigned char *)&ld2;
    for (int i = 0; i < 16; i++) {
        CHECK(50, p[i] == (unsigned char)((0x0102030405060708ull >> (i % 8 * 8)) + i));
        CHECK(51, p[16 + i] == (unsigned char)((0x1112131415161718ull >> (i % 8 * 8)) + i));
    }
    v4f v4 = ret_v4f(0.5f + z);
    CHECK(52, v4[0] == 0.5f && v4[1] == 1.5f && v4[2] == 2.5f && v4[3] == 3.5f);
    v2f v2 = ret_v2f(3.0f + z);
    CHECK(53, v2[0] == 3.0f && v2[1] == 6.0f);
    struct V2 vv = ret_v2(1.0f + z);
    CHECK(54, vv.x[0] == 1 && vv.x[3] == 4 && vv.y[0] == 5 && vv.y[3] == 8);
    struct VM vm = ret_vm(1.0f + z);
    CHECK(55, vm.x[0] == 1 && vm.x[1] == 2 && vm.y[0] == 9 && vm.y[1] == 18);
    struct MX mx = ret_mx(1.5 + z);
    CHECK(56, mx.a == 1.5f && mx.b == 3.0);
    struct F5 f5 = ret_f5(1.0f + z);
    CHECK(57, f5.a == 1 && f5.c == 3 && f5.e == 5);
    struct FI fi = ret_fi(0.75f + z, -11);
    CHECK(58, fi.f == 0.75f && fi.i == -11);
    struct ID id = ret_id(-6 + zero, 2.25);
    CHECK(59, id.i == -6 && id.d == 2.25);
    struct CF cf = ret_cf('z', -0.5f + z);
    CHECK(60, cf.c == 'z' && cf.f == -0.5f);
    struct DS ds = ret_ds(-1.25 + z, -300);
    CHECK(61, ds.d == -1.25 && ds.s == -300);
    struct D1 d1 = ret_d1(8.5 + z);
    CHECK(62, d1.d == 8.5);
    struct D4 s4 = scale_d4(d4, 2.0 + z);
    CHECK(63, s4.a == 8.0 && s4.b == 9.0 && s4.c == 10.0 && s4.d == 11.0);
    struct F3 r3 = rotate_f3(f3);
    CHECK(64, r3.a == 0.25f && r3.b == 8.0f && r3.c == 0.5f);
    /* A returned HFA feeding an HFA argument directly. */
    CHECK(65, sum_d4(scale_d4(ret_d4(1.0 + z), 0.5)) == 0.5 + 1.5 + 4 + 10);
    CHECK(66, call_maker(ret_d3, 1.0 + z) == 1 + 4 + 16);
    return 0;
}

static int check_variadic(void)
{
    const float z = (float)zero;
    struct F3 f3 = {0.5f, 1.0f + z, 2.0f};
    struct D4 d4 = {0.5, 1.5, 2.5 + z, 3.5};
    v4f v4 = {1.0f + z, 0.5f, 0.25f, 2.0f};
    struct D2 d2 = {3.5 + z, 0.125};
    struct V2 vv = {{1, 2, 3, 4 + z}, {0.5f, 0.25f, 0.125f, 1}};
    struct LD1 ld1;
    ld_fill(&ld1.a, 3 + zero);
    struct F5 f5 = {1, 2, 3 + z, 4, 0.5f};
    struct MX mx = {0.5f + z, 4.25};
    struct H3 h3;
    h_fill(&h3.a, 0x3c00 + zero);
    h_fill(&h3.b, 0x4000);
    h_fill(&h3.c, 0xbc00);
    struct NF nf = {{1.0f, 2.0f + z}, 0.5f};
    const double ld1_low = (double)(ld_bits(&ld1.a) & 0xffff);

    /* F3, D4, v4f, double: 3 + 4 + 1 + 1 slots -- the double overflows. */
    CHECK(70, va_hfa(4, 0x4321ull, f3, d4, v4, 0.75) == 10.5 + 2 * 41.5 + 4 * 19.0 + 8 * 0.75);
    /* V2, D2, LD1, F5 (by reference), MX (general registers). */
    CHECK(71, va_hfa(5, 0x98756ull, vv, d2, ld1, f5, mx) ==
                  (49.0 + 16 * 9.5) + 2 * 3.75 + 4 * ld1_low + 8 * 57 + 16 * 9.0);
    /* H3, NF, then D4 needs four slots with only two left: stack. */
    CHECK(72, va_hfa(4, 0x2ba5ull, d2, h3, nf, d4) ==
                  3.75 + 2 * (0x3c00 + 3 * 0x4000 + 9 * 0xbc00) + 4 * 7.0 + 8 * 41.5);
    CHECK(73, va_spill(7, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0 + z, d2, 0.5, 3ll) ==
                  28 + 16 * 3.75 + 128 + 3072);
    struct D3 d3 = {1.0, 2.0 + z, 3.0};
    CHECK(74, va_named_hfa(d3, 0.5, f3, 0.25) == 17 + 4 + 16 * 10.5 + 64);
    struct F2 f2 = {1.5f + z, 0.25f};
    CHECK(75, va_named_overflow(1, 2, 3, 4, 5, 6 + z, d3, 0.5, f2, 7ll) ==
                  21 + 8 * 17 + 32 + 256 + 7168);
    struct DA32 da = {0.5, 0.25 + z, 1.0, 2.0};
    double da_sum = 0.5 + 0.5 + 4 + 16;
    CHECK(76, va_da32(9, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0 + z, da, 0.5) ==
                  45 + 16 * da_sum + 512);
    CHECK(77, va_da32(1, 1.0 + z, da, 0.5) == 1 + 16 * da_sum + 512);
    CHECK(78, da32_after9(1, 2, 3, 4, 5, 6, 7, 8, 9 + z, da, 0.5) == 45 + 16 * da_sum + 512);
    return 0;
}

int main(void)
{
    int r = check_args();
    if (!r)
        r = check_returns();
    if (!r)
        r = check_variadic();
    return r;
}
