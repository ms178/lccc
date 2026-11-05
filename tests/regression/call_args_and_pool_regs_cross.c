/* Call arguments, the static chain and pool registers, executed on AArch64
 * and RV64.
 *
 * AArch64 peephole liveness; each check reduces a gcc.c-torture execute
 * test that main miscompiled:
 * 1. A call modeled as reading no register: an argument staged elsewhere
 *    and copied into x1..x7 just before the call (`movz x0, ...; movk x0,
 *    ...; mov x2, x0`, `mov x1, x10; bl memset`) was deleted as dead
 *    whenever the code after the call ran straight to `ret` (va-arg-6,
 *    20040823-1, 980526-2 and 20050203-1, at -O0 and up).
 * 2. Argument shuffles between calls: an entry copy into an argument
 *    register was coalesced across a call that reads that register
 *    (20000622-1, pr51323, 20000717-5).
 * 3. Every argument class the emitter now annotates on the call (register
 *    pairs, aggregates in GP registers, FP-register and stack arguments, a
 *    memory-returned result through x8), so an annotation naming too few
 *    registers fails here.
 * 4. A loop-carried value in x9 -- an ordinary register in a pure
 *    scalar-ALU function -- lost its definition to a pass that assumed x9
 *    held a block-local address (pr42833 at -O2, -O3 and -Os).
 * 5. The static chain: `mov x18, x0` in front of a call to a nested
 *    function was deleted as dead, so the callee read its parent's frame
 *    through a garbage x18 (nestfunc-7 and nest-align-1 at every level).
 *
 * Expected values: GCC 16.2 -O0 and -O2 and Clang 23.1.5 -O0 and -O2 on
 * x86-64 agree (Clang has no nested functions: check 5 is GCC's), also
 * under -fsanitize=undefined.  The cross harness runs this at -O0, -O1,
 * -O2, -Os and -O3.
 */
#define NOINLINE __attribute__((noinline))
typedef unsigned char u8;
typedef unsigned u32;
typedef long long i64;
typedef signed char i8;
typedef __builtin_va_list va_list;

/* 1. va-arg-6: wide constants between narrow ones. */
NOINLINE i64 vsum(int n, ...)
{
    va_list ap;
    __builtin_va_start(ap, n);
    i64 s = 0;
    for (int i = 0; i < n; i++)
        s = s * 3 + ((i & 1) ? __builtin_va_arg(ap, i64) : __builtin_va_arg(ap, int));
    __builtin_va_end(ap);
    return s;
}
NOINLINE i64 wide_args(void)
{
    return vsum(8, 10, 10000000000LL, 11, 20000000000LL, 12, 30000000000LL, 13, 40000000000LL)
           + 1;
}
NOINLINE i64 wide_args_negative(void)
{
    return vsum(5, -1, -4294967296LL, 7, 0x123456789abcLL, -9) - 1;
}
/* 20040823-1: constant arguments routed through scratch registers. */
NOINLINE void fill(u8 *p, int v, int n)
{
    for (int i = 0; i < n; i++)
        p[i] = (u8)(v + i);
}
NOINLINE int fill_then_read(void)
{
    u8 buf[8] = { 9, 9, 9, 9, 9, 9, 9, 9 };
    fill(buf, 0, 4);
    return buf[0] + buf[3] * 10 + buf[4] * 100;
}
/* 980526-2: a value computed before the call, passed in x2. */
NOINLINE int mknod_dev(const char *name, int mode, u32 dev) { return name[0] + mode + (int)(dev >> 20); }
NOINLINE int sys_mknod(const char *name, int mode, u32 dev)
{
    return mknod_dev(name, mode, ((dev >> 8) << 22) | (dev & 0xff)) * 2;
}
/* 20050203-1: an argument derived from a value loaded after a call. */
NOINLINE void set81(u8 *p) { *p = 0x81; }
NOINLINE int twice(int x) { return x * 2; }
NOINLINE int masked_arg(void)
{
    u8 b;
    set81(&b);
    return twice(b & 0x7f) + 1;
}

/* 2. Shuffles: 20000622-1, pr51323, 20000717-5. */
NOINLINE long sh_foo(long a, long b, long c) { return a * 10000 + b * 100 + c; }
NOINLINE long sh_bar(long a, long b) { return b + (a & 0); }
NOINLINE long sh_baz(long a, long b, void *c)
{
    long d = (long)c;
    return sh_foo(d, sh_bar(a, 1), b);
}
struct S { int a, b, c; };
NOINLINE int p_foo(int x, int y, int z) { return x * 10000 + y * 100 + z; }
NOINLINE int p_bar(int x, struct S y) { return p_foo(y.b, 0, x); }
typedef struct { int a, b, c; } trio;
NOINLINE int t_bar(int i, int j, int k, trio t)
{
    return ((t.a * 10 + t.b) * 10 + t.c) * 1000 + i * 100 + j * 10 + k;
}
NOINLINE int t_foo(trio t, int i, int j, int k) { return t_bar(i, j, k, t); }

/* 3. Argument classes. */
NOINLINE i64 ten(i64 a, i64 b, i64 c, i64 d, i64 e, i64 f, i64 g, i64 h, i64 i, i64 j)
{
    return a + 2 * b + 3 * c + 5 * d + 7 * e + 11 * f + 13 * g + 17 * h + 19 * i + 23 * j;
}
NOINLINE i64 ten_wide(void)
{
    return ten(0x100000001LL, 0x200000002LL, 3, 4, 5, 6, 7, 8, 0x900000009LL, 10) + 1;
}
struct pair { i64 lo, hi; };
struct big { i64 v[3]; };
struct hfa { double x, y; };
NOINLINE i64 mixed(int a, __int128 w, struct pair p, double d, struct hfa h, int z)
{
    return a + (i64)(w >> 64) + (i64)w + p.lo - p.hi + (i64)(d * 4) + (i64)(h.x - h.y) + z;
}
NOINLINE i64 mixed_call(void)
{
    struct pair p = { 1000, 7 };
    struct hfa h = { 9.5, 2.5 };
    __int128 w = ((__int128)5 << 64) | 6;
    return mixed(3, w, p, 2.25, h, -4) * 2;
}
NOINLINE struct big make_big(i64 a, i64 b)
{
    struct big r = { { a, b, a * b } };
    return r;
}
NOINLINE i64 big_call(i64 a)
{
    struct big b = make_big(a, 7);
    return b.v[0] + b.v[1] * 100 + b.v[2] * 10000;
}

/* 4. pr42833's helper_neon_rshl_s8, statement for statement: the shape of
 * its control flow is what put the loop-carried value in x9. */
typedef struct { i8 v1; i8 v2; i8 v3; i8 v4; } neon_s8;
typedef int ssize_t;
#define RSHL_LANE(lane)                                                      \
    do {                                                                    \
        i8 tmp;                                                             \
        tmp = (i8)vsrc2.lane;                                               \
        if (tmp >= (ssize_t)sizeof(vsrc1.lane) * 8) {                       \
            vdest.lane = 0;                                                 \
        } else if (tmp < -(ssize_t)sizeof(vsrc1.lane) * 8) {                \
            vdest.lane = vsrc1.lane >> (sizeof(vsrc1.lane) * 8 - 1);        \
        } else if (tmp == -(ssize_t)sizeof(vsrc1.lane) * 8) {               \
            vdest.lane = vsrc1.lane >> (tmp - 1);                           \
            vdest.lane++;                                                   \
            vdest.lane >>= 1;                                               \
        } else if (tmp < 0) {                                               \
            vdest.lane = (vsrc1.lane + (1 << (-1 - tmp))) >> -tmp;          \
        } else {                                                            \
            vdest.lane = vsrc1.lane << tmp;                                 \
        }                                                                   \
    } while (0)
NOINLINE u32 rshl_s8(u32 arg1, u32 arg2)
{
    u32 res;
    neon_s8 vsrc1;
    neon_s8 vsrc2;
    neon_s8 vdest;
    do {
        union { neon_s8 v; u32 i; } conv_u;
        conv_u.i = (arg1);
        vsrc1 = conv_u.v;
    } while (0);
    do {
        union { neon_s8 v; u32 i; } conv_u;
        conv_u.i = (arg2);
        vsrc2 = conv_u.v;
    } while (0);
    RSHL_LANE(v1);
    RSHL_LANE(v2);
    RSHL_LANE(v3);
    RSHL_LANE(v4);
    do {
        union { neon_s8 v; u32 i; } conv_u;
        conv_u.v = (vdest);
        res = conv_u.i;
    } while (0);
    return res;
}

/* 5. nestfunc-7: nested functions reading the parent's locals through the
 * static chain.  Without arguments, the chain is computed in x0 and copied
 * to x18 -- the copy that was lost; `grow0` also returns its result in
 * memory (x8 on AArch64), as nestfunc-7's `Foo` does. */
NOINLINE int chained(int base)
{
    int scale = base * 2;
    struct big NOINLINE grow0(void)
    {
        struct big r = { { base + 1, scale + 2, base * scale } };
        return r;
    }
    struct big NOINLINE grow(i64 k)
    {
        struct big r = { { base + k, scale + k, base * scale + k } };
        return r;
    }
    int NOINLINE sum0(void) { return base + scale; }
    int NOINLINE pick(int i) { return i ? scale : base; }
    struct big g0 = grow0();
    struct big g = grow(5);
    return (int)(g.v[0] + 2 * g.v[1] + 3 * g.v[2]) + pick(0) * 100 + pick(1) * 1000
           + (int)(g0.v[0] * 10000 + g0.v[1] * 100000 + g0.v[2] * 1000000) + sum0() * 10;
}

/* nestfunc-7 itself, renamed: the first nested call of the function, so x18
 * holds nothing useful when the lost copy should have set it. */
struct A6 { int one, two, three, four, five, six; };
NOINLINE int nest7(void)
{
    int base;
    struct A6 Foo(void)
    {
        struct A6 a;
        a.one = base + 1;
        a.two = base + 2;
        a.three = base + 3;
        a.four = base + 4;
        a.five = base + 5;
        a.six = base + 6;
        return a;
    }
    base = 10;
    struct A6 a = Foo();
    return a.one == 11 && a.two == 12 && a.three == 13 && a.four == 14 && a.five == 15
           && a.six == 16;
}

/* Inputs the compiler cannot see: LCCC evaluates calls of pure functions
 * with constant arguments at compile time, which would leave nothing of a
 * check to execute. */
static volatile int in_int[8] = { 10, 11, 12, 9, 2, 3, 4, 1 };
static volatile u32 in_u32[5] = { 0x12345678, 0x05050505, 0x01010101, 0x80f81144, 0xfe09f701 };
static volatile i64 in_i64 = 6;
static const char name[] = "test";
static const char *volatile in_name = name;

int main(void)
{
    if (wide_args() != 9220000024907LL || wide_args_negative() != 59932030914584LL)
        return 1;
    if (fill_then_read() != 930 || sys_mknod(in_name, in_int[7], in_u32[0]) != 922)
        return 1;
    if (masked_arg() != 3)
        return 1;
    if (sh_baz(in_int[0], in_int[1], (void *)(long)in_int[2]) != 120111)
        return 2;
    struct S s = { in_int[4], in_int[5], in_int[6] };
    if (p_bar(in_int[3], s) != 30009)
        return 2;
    trio t = { in_int[7], in_int[4], in_int[5] };
    if (t_foo(t, in_int[6], in_int[6] + 1, in_int[6] + 2) != 123456)
        return 2;
    if (ten_wide() != 755914244860LL || mixed_call() != 2038 || big_call(in_i64) != 420706)
        return 3;
    if (rshl_s8(in_u32[1], in_u32[2]) != 0x0a0a0a0a)
        return 4;
    /* Lanes: 0x44 << 1, 0x11 >> 9 (below -8: sign fill), -8 << 9 (8 or
     * more: zero), -128 rounded >> 2.  No input takes the original's
     * shift-by-(tmp - 1) path, which is undefined for tmp == -8. */
    if (rshl_s8(in_u32[3], in_u32[4]) != 0xe0000088)
        return 4;
    if (chained(in_int[7] + 2) != 18846489 || nest7() != 1)
        return 5;
    return 0;
}
