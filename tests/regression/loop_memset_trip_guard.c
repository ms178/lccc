/* Fill loops that loop_memset turns into memset(3), across the trip-count
 * space: unsigned and signed bounds, init above/below/equal to the bound,
 * negative values, != exits, do-while forms, 1/4-byte elements and live-out
 * induction variables. The guarded call branches on `init < bound` and
 * passes `bound - init` (the clamp `select(init < bound, bound - init, 0)`
 * is kept only for exit values); a zero init passes the bound itself.
 * Compared against GCC by the regression corpus. */
#include <stdio.h>
#include <string.h>

#define N 96

static unsigned char buf[N + 8];
static int ibuf[N + 8];

__attribute__((noinline)) static void fill_ult(unsigned char *d, unsigned long a, unsigned long n)
{
    for (unsigned long i = a; i < n; i++)
        d[i] = 0x5a;
}

__attribute__((noinline)) static unsigned long fill_ult_live(unsigned char *d, unsigned long a,
                                                             unsigned long n)
{
    unsigned long i;
    for (i = a; i < n; i++)
        d[i] = 0x11;
    return i;
}

__attribute__((noinline)) static void fill_u32(unsigned char *d, unsigned a, unsigned n)
{
    for (unsigned i = a; i < n; i++)
        d[i] = 0x22;
}

__attribute__((noinline)) static void fill_slt(unsigned char *d, int a, int n)
{
    for (int i = a; i < n; i++)
        d[i] = 0x33;
}

__attribute__((noinline)) static int fill_slt_live(unsigned char *d, int a, int n)
{
    int i;
    for (i = a; i < n; i++)
        d[i] = 0x44;
    return i;
}

__attribute__((noinline)) static void fill_int0(int *d, int n)
{
    for (int i = 0; i < n; i++)
        d[i] = 0;
}

__attribute__((noinline)) static void fill_int_from(int *d, int a, int n)
{
    for (int i = a; i < n; i++)
        d[i] = -1;
}

__attribute__((noinline)) static void fill_ne(unsigned char *d, unsigned long a, unsigned long n)
{
    for (unsigned long i = a; i != n; i++)
        d[i] = 0x55;
}

__attribute__((noinline)) static void fill_ptr(unsigned char *p, unsigned char *e)
{
    while (p < e)
        *p++ = 0x66;
}

__attribute__((noinline)) static unsigned long fill_do(unsigned char *d, unsigned long a,
                                                       unsigned long n)
{
    unsigned long i = a;
    do
        d[i] = 0x77;
    while (++i < n);
    return i;
}

static unsigned hash(void)
{
    unsigned h = 2166136261u;
    for (unsigned k = 0; k < sizeof buf; k++)
        h = (h ^ buf[k]) * 16777619u;
    for (unsigned k = 0; k < N + 8; k++)
        h = (h ^ (unsigned)ibuf[k]) * 16777619u;
    return h;
}

static void reset(void)
{
    memset(buf, 0, sizeof buf);
    memset(ibuf, 0, sizeof ibuf);
}

int main(void)
{
    static const int pts[] = {-5, -1, 0, 1, 2, 7, 31, 64, 90};
    const unsigned np = sizeof pts / sizeof pts[0];
    unsigned acc = 0;
    for (unsigned x = 0; x < np; x++) {
        for (unsigned y = 0; y < np; y++) {
            int a = pts[x], n = pts[y];
            if (a >= 0 && n >= 0) {
                reset();
                fill_ult(buf, (unsigned long)a, (unsigned long)n);
                acc = acc * 31u + hash();
                reset();
                acc = acc * 31u + (unsigned)fill_ult_live(buf, (unsigned long)a, (unsigned long)n);
                acc = acc * 31u + hash();
                reset();
                fill_u32(buf, (unsigned)a, (unsigned)n);
                acc = acc * 31u + hash();
                reset();
                fill_ptr(buf + a, buf + n);
                acc = acc * 31u + hash();
                if (a < n) {
                    reset();
                    fill_ne(buf, (unsigned long)a, (unsigned long)n);
                    acc = acc * 31u + hash();
                }
                if (a < N) {
                    reset();
                    acc = acc * 31u + (unsigned)fill_do(buf, (unsigned long)a, (unsigned long)n);
                    acc = acc * 31u + hash();
                }
            }
            /* Signed forms: the index may be negative only when no store
             * happens or the base is offset accordingly. */
            reset();
            fill_slt(buf + 8, a, n);
            acc = acc * 31u + hash();
            reset();
            acc = acc * 31u + (unsigned)fill_slt_live(buf + 8, a, n);
            acc = acc * 31u + hash();
            reset();
            fill_int_from(ibuf + 8, a, n);
            acc = acc * 31u + hash();
            reset();
            fill_int0(ibuf, n);
            acc = acc * 31u + hash();
        }
    }
    printf("%08x\n", acc);
    return 0;
}
