/* loop_memset may turn `for (i = a; i < n; i++) d[f(i)] = c;` into one
 * memset only when f is exactly affine in i over the visited range, and the
 * memset must start where the loop's first store went. Each function here
 * probes one way that fails or once failed:
 *   - narrowing casts of the counter wrap the index ((unsigned char)i,
 *     (unsigned short)i, (signed char)i), so a single memset of n elements
 *     wrote past the object;
 *   - an extension that does not match the counter's signedness;
 *   - a runtime, possibly negative start (the start offset must be
 *     extended exactly as the loop extends it: at -Os, without IV widening,
 *     the raw 32-bit register reached the address);
 *   - scaled and offset indices (p[i], d[i + 2], d[i - 1], p[i + 3]),
 *     narrow (short, unsigned char) and 64-bit counters (truncated to
 *     address width on i686);
 *   - counters that wrap in their own type: != exits whose entry value is
 *     above the bound (short 32767 -> -32768, unsigned char 255 -> 0,
 *     unsigned short 65535 -> 0) and a second counter beside the exit
 *     counter; the extended address jumps back, so no single memset is
 *     exact (an unsigned int counter wraps the same way on LP64, but only
 *     an object over 4 GiB can show it);
 *   - != trip counts are modular differences: zero-extended, never
 *     sign-extended.
 * Every buffer is hashed in full, including guard bytes on both sides.
 * Built at -O2 here and at -Os / -m32 -Os by the _Os / _i686 wrappers;
 * compared against GCC by the regression corpus. */
#include <stdio.h>
#include <string.h>

#define NB 1024
#define NW 70016

static unsigned char buf[NB];
static unsigned char wide[NW];
static int ibuf[256];
static unsigned h;

static void mix(const void *p, size_t n)
{
    const unsigned char *b = p;
    for (size_t k = 0; k < n; k++)
        h = h * 31u + b[k];
}

static void reset(void)
{
    memset(buf, 0, sizeof buf);
    memset(ibuf, 0, sizeof ibuf);
}

__attribute__((noinline)) static void f_u8(unsigned char *d, int n)
{
    for (int i = 0; i < n; i++)
        d[(unsigned char)i] = 9;
}

__attribute__((noinline)) static void f_s8(unsigned char *d, int n)
{
    for (int i = 0; i < n; i++)
        d[(signed char)i + 128] = 8;
}

__attribute__((noinline)) static void f_u16(unsigned char *d, unsigned long n)
{
    for (unsigned long i = 0; i < n; i++)
        d[(unsigned short)i] = 7;
}

__attribute__((noinline)) static void f_zext(unsigned char *d, int a, int n)
{
    for (int i = a; i < n; i++)
        d[(unsigned)i] = 6;
}

__attribute__((noinline)) static void f_slt(unsigned char *d, int a, int n)
{
    for (int i = a; i < n; i++)
        d[i] = 0x33;
}

__attribute__((noinline)) static void f_int_scaled(int *p, int a, int n)
{
    for (int i = a; i < n; i++)
        p[i] = 0;
}

__attribute__((noinline)) static void f_int_scaled_nz(int *p, int a, int n)
{
    for (int i = a; i < n; i++)
        p[i] = -1;
}

__attribute__((noinline)) static void f_plus2(unsigned char *d, int a, int n)
{
    for (int i = a; i < n; i++)
        d[i + 2] = 0x5c;
}

__attribute__((noinline)) static void f_minus1(unsigned char *d, int a, int n)
{
    for (int i = a; i < n; i++)
        d[i - 1] = 0x4d;
}

__attribute__((noinline)) static void f_int_plus3(int *p, int a, int n)
{
    for (int i = a; i < n; i++)
        p[i + 3] = 0;
}

__attribute__((noinline)) static void f_u32_plus2(unsigned char *d, unsigned a, unsigned n)
{
    for (unsigned i = a; i < n; i++)
        d[i + 2] = 0x2b;
}

__attribute__((noinline)) static void f_short(unsigned char *d, short a, short n)
{
    for (short i = a; i < n; i++)
        d[i] = 0x1e;
}

__attribute__((noinline)) static void f_uchar(unsigned char *d, unsigned char a, unsigned char n)
{
    for (unsigned char i = a; i < n; i++)
        d[i] = 0x2e;
}

__attribute__((noinline)) static void f_ll(unsigned char *d, long long a, long long n)
{
    for (long long i = a; i < n; i++)
        d[i] = 0x3e;
}

__attribute__((noinline)) static void f_ull_int(int *p, unsigned long long a, unsigned long long n)
{
    for (unsigned long long i = a; i < n; i++)
        p[i] = 0;
}

__attribute__((noinline)) static void f_two(unsigned char *d, unsigned a, unsigned n)
{
    unsigned j = a;
    for (unsigned i = 0; i < n; i++, j++)
        d[j] = 0x61;
}

__attribute__((noinline)) static void f_two_s(unsigned char *d, int a, int n)
{
    int j = a;
    for (int i = 0; i < n; i++, j++)
        d[j] = 0x62;
}

__attribute__((noinline)) static void f_two_l(unsigned char *d, long a, unsigned n)
{
    long j = a;
    for (unsigned i = 0; i < n; i++, j++)
        d[j] = 0x63;
}

__attribute__((noinline)) static void f_ne_s(unsigned char *d, int a, int n)
{
    for (int i = a; i != n; i++)
        d[i] = 0x64;
}

__attribute__((noinline)) static void f_ne_u32(unsigned char *d, unsigned a, unsigned n)
{
    for (unsigned i = a; i != n; i++)
        d[i] = 0x65;
}

__attribute__((noinline)) static void f_ne_u32_0(unsigned char *d, unsigned n)
{
    for (unsigned i = 0; i != n; i++)
        d[i] = 0x66;
}

__attribute__((noinline)) static void f_ne_dowhile(unsigned char *d, unsigned a, unsigned n)
{
    unsigned i = a;
    do
        d[i] = 0x67;
    while (++i != n);
}

__attribute__((noinline)) static void f_ne_short(unsigned char *d, short a, short n)
{
    for (short i = a; i != n; i++)
        d[i] = 0x68;
}

__attribute__((noinline)) static void f_ne_uchar(unsigned char *d, unsigned char a, unsigned char n)
{
    for (unsigned char i = a; i != n; i++)
        d[i] = 0x69;
}

__attribute__((noinline)) static void f_ne_ushort(unsigned char *d, unsigned short a,
                                                  unsigned short n)
{
    for (unsigned short i = a; i != n; i++)
        d[i] = 0x6a;
}

/* Live-out narrow counters: the exit value is rebuilt in the counter's own
 * type. */
__attribute__((noinline)) static int f_short_live(unsigned char *d, short a, short n)
{
    short i;
    for (i = a; i < n; i++)
        d[i] = 0x6b;
    return i;
}

int main(void)
{
    static const int ranges[][2] = {
        {0, 0}, {0, 1}, {0, 37}, {5, 5}, {5, 4}, {-17, 23}, {-40, -3},
        {-1, 0}, {3, 40}, {-64, 64}, {10, -10}, {-2, -1},
    };
    const int nr = (int)(sizeof ranges / sizeof ranges[0]);

    reset();
    f_u8(buf, 300);
    mix(buf, sizeof buf);
    reset();
    f_u8(buf, 256);
    mix(buf, sizeof buf);
    reset();
    f_s8(buf, 200);
    mix(buf, sizeof buf);
    reset();
    f_s8(buf, 128);
    mix(buf, sizeof buf);

    memset(wide, 0, sizeof wide);
    f_u16(wide + 64, 70000);
    mix(wide, sizeof wide);
    memset(wide, 0, sizeof wide);
    f_u16(wide + 64, 65535);
    mix(wide, sizeof wide);

    for (int r = 0; r < nr; r++) {
        int a = ranges[r][0], n = ranges[r][1];

        reset();
        if (a >= 0)
            f_zext(buf + 128, a, n);
        f_slt(buf + 512, a, n);
        mix(buf, sizeof buf);

        reset();
        f_int_scaled(ibuf + 100, a, n);
        mix(ibuf, sizeof ibuf);
        reset();
        for (int k = 0; k < 256; k++)
            ibuf[k] = k;
        f_int_scaled(ibuf + 100, a, n);
        mix(ibuf, sizeof ibuf);
        reset();
        f_int_scaled_nz(ibuf + 100, a, n);
        mix(ibuf, sizeof ibuf);

        reset();
        f_plus2(buf + 300, a, n);
        f_minus1(buf + 600, a, n);
        f_short(buf + 800, (short)a, (short)n);
        f_ll(buf + 200, a, n);
        mix(buf, sizeof buf);

        reset();
        f_int_plus3(ibuf + 120, a, n);
        mix(ibuf, sizeof ibuf);

        if (a >= 0 && n >= 0) {
            reset();
            f_u32_plus2(buf + 40, (unsigned)a, (unsigned)n);
            f_uchar(buf + 400, (unsigned char)a, (unsigned char)n);
            mix(buf, sizeof buf);
            reset();
            f_ull_int(ibuf + 8, (unsigned long long)a, (unsigned long long)n);
            mix(ibuf, sizeof ibuf);
        }
    }

    for (int r = 0; r < nr; r++) {
        int a = ranges[r][0], n = ranges[r][1];

        reset();
        f_two_s(buf + 500, a, n > 0 ? n : 0);
        f_two_l(buf + 700, a, n > 0 ? (unsigned)n : 0u);
        if (a >= 0)
            f_two(buf + 100, (unsigned)a, n > 0 ? (unsigned)n : 0u);
        mix(buf, sizeof buf);

        reset();
        if (a <= n) {
            f_ne_s(buf + 300, a, n);
            h = h * 31u + (unsigned)f_short_live(buf + 600, (short)a, (short)n);
        }
        if (0 <= a && a <= n) {
            f_ne_u32(buf + 64, (unsigned)a, (unsigned)n);
            f_ne_short(buf + 400, (short)a, (short)n);
        }
        if (0 <= a && a < n)
            f_ne_dowhile(buf + 800, (unsigned)a, (unsigned)n);
        if (n >= 0)
            f_ne_u32_0(buf + 900, (unsigned)n);
        mix(buf, sizeof buf);
        h = h * 31u + (unsigned)f_short_live(buf + 600, (short)a, (short)n);
    }

    /* Wrapping != counters: both halves of the range land inside `wide`. */
    memset(wide, 0, sizeof wide);
    f_ne_short(wide + 32800, 32760, -32760);
    f_ne_short(wide + 32800, 5, 5);
    mix(wide, sizeof wide);
    memset(wide, 0, sizeof wide);
    f_ne_ushort(wide + 16, 65530, 7);
    f_ne_ushort(wide + 16, 100, 90);
    mix(wide, sizeof wide);
    reset();
    f_ne_uchar(buf + 8, 250, 5);
    f_ne_uchar(buf + 400, 3, 3);
    f_ne_uchar(buf + 600, 7, 200);
    f_ne_uchar(buf + 300, 200, 7);
    mix(buf, sizeof buf);

    printf("%08x\n", h);
    return 0;
}
