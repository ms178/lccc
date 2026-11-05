/* AArch64 and RISC-V leaf, parameter and FP-domain contracts, executed.
 *
 * AArch64:
 * - A call-free leaf whose ninth parameter arrives on the stack keeps no
 *   frame: the parameter is read from [sp, #offset] at entry (`nine`,
 *   `ten`, the narrow `nine_u8`/`nine_i16`).
 * - A parameter arriving in a pool register (x4-x7) is homed where it
 *   arrives, and the temporaries must leave it alone until its last read
 *   (`six`: `a + b*c` once took x5 while `f` still lived there).
 * - `double f(double d, int k)`: the ParamRef bounce through x0 destroyed
 *   `k` -- the first integer parameter, still unread in x0 (`scale`).
 * - Selects with FP-register operands are `fcsel`, conversions between FP
 *   registers are `fcvt`/`fcvtzs`/`fcvtzu` register to register, with
 *   constants, homes and narrow results on either side.
 *
 * RISC-V:
 * - Narrow parameters are used in their psABI register form (no entry
 *   re-extension), the prologue publishes the argument extensions, and the
 *   upper-word demand pass drops zero extensions only W instructions or
 *   sign extensions observe -- every value below crosses those boundaries
 *   with its sign bit set.
 *
 * Expected values are GCC 16.2 -O2 and Clang 23.1.4 -O0 on x86-64 (they
 * agree).  The cross harness runs this on AArch64 and RV64 at every level.
 */
#define NOINLINE __attribute__((noinline))
typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned u32;
typedef unsigned long long u64;
typedef signed char i8;
typedef short i16;
typedef long long i64;

NOINLINE int six(int a, int b, int c, int d, int e, int f) { return a + b * c - d + e * f; }
NOINLINE int nine(int a, int b, int c, int d, int e, int f, int g, int h, int i)
{
    return a - b + c - d + e - f + g - h + i * 3;
}
NOINLINE long ten(long a, long b, long c, long d, long e, long f, long g, long h, long i, long j)
{
    return (a ^ b) + (c ^ d) + (e ^ f) + (g ^ h) + i * j;
}
NOINLINE u8 nine_u8(int a, int b, int c, int d, int e, int f, int g, int h, u8 i)
{
    return (u8)(i + a + h);
}
NOINLINE i16 nine_i16(int a, int b, int c, int d, int e, int f, int g, int h, i16 i)
{
    return (i16)(i - h);
}
NOINLINE double scale(double d, int k) { return d * k + k; }
NOINLINE float scalef(float d, int k, float e) { return d * k - e; }
NOINLINE double pick(double a, double b, int k) { return k ? a : b; }
NOINLINE float pickf(float a, float b, int k) { return k ? a : b; }
NOINLINE double pick_lt(double a, double b, long x, long y) { return x < y ? a : b; }
NOINLINE double pick_const(double a, int k) { return k ? a : 2.5; }
NOINLINE float to_float(double d) { return (float)d; }
NOINLINE double to_double(float f) { return (double)f + 1.0; }
NOINLINE float fmulf(float a, float b)
{
    double t = a;
    return (float)(t * b) + a;
}
NOINLINE int to_int(double d) { return (int)(d * 2.0); }
NOINLINE u8 to_u8(float f) { return (u8)(f + 1.0f); }
NOINLINE i16 to_i16(double d) { return (i16)(d - 1.0); }
NOINLINE u32 to_u32(double d) { return (u32)(d * 3.0); }
NOINLINE i64 to_i64(float f) { return (i64)(f * 1024.0f); }
NOINLINE u64 to_u64(double d) { return (u64)(d * 4.0); }
NOINLINE int par16(u16 x) { return __builtin_parity(x); }
NOINLINE int pc8(u8 x) { return __builtin_popcount(x); }
NOINLINE int pc16(u16 x) { return __builtin_popcount(x); }
NOINLINE int par32(u32 x) { return __builtin_parity(x); }
NOINLINE u32 bset32(u32 x, u32 n) { return x | (1u << n); }
NOINLINE u64 shl1(u32 n) { return (u64)(1u << n); }
NOINLINE long widen_i8(i8 x, i16 y, int z) { return (long)x * 3 + y - z; }
NOINLINE u64 widen_u(u8 x, u16 y, u32 z) { return ((u64)x << 40) | ((u64)y << 20) | z; }
NOINLINE u32 wrap_add(u32 a, u32 b) { return a + b; }
NOINLINE int lt_after_wrap(u32 a, u32 k)
{
    u32 b = a + k;
    return a < b;
}
NOINLINE u32 sum_u32(const u32 *p, u32 n)
{
    u32 s = 0;
    for (u32 i = 0; i < n; i++)
        s += p[i] ^ i;
    return s;
}
NOINLINE u64 mix_u32(u32 a, u32 b)
{
    u32 t = a * 0x9e3779b9u + b;
    return (u64)t * 3u;
}

static u64 bits(double d)
{
    union { double d; u64 u; } v;
    v.d = d;
    return v.u;
}
static u32 bitsf(float f)
{
    union { float f; u32 u; } v;
    v.f = f;
    return v.u;
}

/* Every argument comes from memory, so no interprocedural constant reaches
 * a callee. */
static volatile int vi[10] = {3, -7, 11, 13, -17, 19, 7, 8, -9, 0};
static volatile long vl[10] = {1, 2, 3, 4, 5, 6, 7, 8, 100000, -300000};
static volatile double vd[9] = {1.75, 1.5, -2.25, 3.0, 4.0, -0.0, 1.1, -12.75, 4.6e18};
static volatile float vf[7] = {0.5f, 1.25f, 1.5f, -2.25f, 0.1f, 1.1f, 3.3f};
static volatile u32 vu[6] = {0x80000001u, 0xffffffffu, 0x80000000u, 31, 5, 0xdeadbeefu};
static u32 arr[5] = {0x80000001u, 7, 0xfffffff0u, 12345, 0x7fffffffu};

int main(void)
{
    int k1 = vi[9] + 1, k0 = vi[9];
    if (six(vi[0], vi[1], vi[2], vi[3], vi[4], vi[5]) != -410) return 1;
    if (nine(1, 2, vi[0], 4, 5, 6, vi[6], vi[7], vi[8]) != -31) return 2;
    if (ten(vl[0], vl[1], vl[2], vl[3], vl[4], vl[5], vl[6], vl[7], vl[8], vl[9]) != -29999999972L)
        return 3;
    if (nine_u8(vi[9] + 250, 0, 0, 0, 0, 0, 0, vi[8] + 18, (u8)(vu[4] + 195)) != 203) return 4;
    if (nine_i16(0, 0, 0, 0, 0, 0, 0, vi[9] + 40000, (i16)(vi[9] - 32000)) != -6464) return 5;
    if (bits(scale(vd[0], vi[9] - 3)) != 0xc020800000000000ull) return 6;
    if (bitsf(scalef(vf[0], vi[6], vf[1])) != 0x40100000u) return 7;
    if (bits(pick(vd[1], vd[2], k1)) != 0x3ff8000000000000ull) return 8;
    if (bits(pick(vd[1], vd[2], k0)) != 0xc002000000000000ull) return 9;
    if (bitsf(pickf(vf[2], vf[3], k1 + 4)) != 0x3fc00000u) return 10;
    if (bitsf(pickf(vf[2], vf[3], k0)) != 0xc0100000u) return 11;
    if (bits(pick_lt(vd[3], vd[4], vi[9] - 5, vi[9] + 2)) != 0x4008000000000000ull) return 12;
    if (bits(pick_lt(vd[3], vd[4], vi[9] + 5, vi[9] + 2)) != 0x4010000000000000ull) return 13;
    if (bits(pick_const(vd[5], k1)) != 0x8000000000000000ull) return 14;
    if (bits(pick_const(vd[5], k0)) != 0x4004000000000000ull) return 15;
    if (bitsf(to_float(vd[6])) != 0x3f8ccccdu) return 16;
    if (bits(to_double(vf[4])) != 0x3ff199999a000000ull) return 17;
    if (bitsf(fmulf(vf[5], vf[6])) != 0x40975c29u) return 18;
    if (to_int(vd[7]) != -25) return 19;
    if (to_u8(vf[0] + 200.0f) != 201) return 20;
    if (to_i16(vd[0] - 30002.25) != -30001) return 21;
    if (to_u32(vd[1] + 999999999.0) != 3000000001u) return 22;
    if (to_i64(vf[0] - 123457.0f) != -126419456LL) return 23;
    if (to_u64(vd[8]) != 18400000000000000000ull) return 24;
    if (par16((u16)(vu[1] & 0xffff)) != 0 || par16((u16)((vu[0] >> 16 | 1) ^ 0x100)) != 1) return 25;
    if (pc8((u8)vu[1]) != 8 || pc8((u8)(vu[0] | 0x80)) != 2) return 26;
    if (pc16((u16)vu[1]) != 16 || pc16((u16)(vu[2] >> 16 | 0x0421)) != 4) return 27;
    if (par32(vu[2]) != 1 || par32(vu[1]) != 0) return 28;
    if (bset32(vu[2], vi[9]) != 2147483649u || bset32(vu[0] & 1, vu[3]) != 2147483649u) return 29;
    if (shl1(vu[3]) != 2147483648ull || shl1(vu[4]) != 32) return 30;
    if (widen_i8((i8)(vi[9] - 128), (i16)(vi[9] - 32768), vi[9] - 2147483647 - 1) != 2147450496L)
        return 31;
    if (widen_u((u8)vu[1], (u16)vu[1], vu[1]) != 280444184559615ull) return 32;
    if (wrap_add(vu[0], vu[1]) != 2147483648u) return 33;
    if (lt_after_wrap(vu[0], vu[1]) != 0 || lt_after_wrap(vu[4], vu[4] + 1) != 1) return 34;
    if (sum_u32(arr, vu[4]) != 12334u) return 35;
    if (mix_u32(vu[5], vu[2]) != 913626405ull) return 36;
    return 0;
}
