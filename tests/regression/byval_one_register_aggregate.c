/* An aggregate of one INTEGER eightbyte is passed as an integer of its size
 * (x86-64 psABI 3.2.3): `struct bw_fixed { int64_t value; }`, `pte_t`,
 * `kuid_t`, `struct fixed31_32`.  LCCC built a stack image for each such
 * argument that was not already in memory, one frame slot per call:
 * amdgpu's `calculate_bandwidth` had a 7832-byte frame against GCC's 240.
 *
 * The image is now the value itself when it only carries it.  This file
 * checks the values through every argument position, the stack, variadic
 * calls, function pointers, recursion and aggregates of 1, 2, 4 and 8 bytes;
 * the two-compiler ABI check and the frame are in
 * tests/regression/check_kernel_driver_shapes.py.
 */
#include <stdarg.h>
#include <stdio.h>

struct fixed {
    long long value;
};
struct word {
    int v;
};
struct half {
    short v;
};
struct byte {
    unsigned char v;
};
struct ptr {
    const char *p;
};
struct pair {
    long a, b;
};

#define NOINLINE __attribute__((noinline))

NOINLINE struct fixed fx(long long v)
{
    struct fixed r = { v };
    return r;
}
NOINLINE struct fixed mul(struct fixed a, struct fixed b) { return fx(a.value * b.value); }
NOINLINE struct fixed sub(struct fixed a, struct fixed b) { return fx(a.value - b.value); }
static inline struct fixed add(struct fixed a, struct fixed b) { return fx(a.value + b.value); }
static inline struct fixed max2(struct fixed a, struct fixed b) { return a.value > b.value ? a : b; }

/* Every register position, then the stack: the seventh integer argument on. */
NOINLINE long long spread(struct fixed a, long b, struct fixed c, int d, struct fixed e,
                          struct fixed f, struct fixed g, long h, struct fixed i)
{
    return a.value + 2 * b + 3 * c.value + 4 * d + 5 * e.value + 6 * f.value + 7 * g.value +
           8 * h + 9 * i.value;
}
NOINLINE int narrow(struct byte a, struct half b, struct word c, struct ptr d, struct byte e)
{
    return a.v + b.v * 3 + c.v * 5 + d.p[1] * 7 + e.v * 11;
}
NOINLINE long long sum_fixed(int n, ...)
{
    va_list ap;
    long long s = 0;
    va_start(ap, n);
    for (int k = 0; k < n; k++)
        s = s * 3 + va_arg(ap, struct fixed).value;
    va_end(ap);
    return s;
}
NOINLINE long with_pair(struct fixed a, struct pair p, struct fixed b)
{
    return a.value * 1000 + p.a * 100 + p.b * 10 + b.value;
}
NOINLINE struct fixed fold(struct fixed acc, int depth)
{
    if (depth == 0)
        return acc;
    return fold(add(mul(acc, fx(3)), fx(depth)), depth - 1);
}

/* The shape of `calculate_bandwidth`: each temporary feeds the next call. */
NOINLINE long long bandwidth(const struct fixed *in, int n)
{
    struct fixed total = fx(0);
    for (int k = 0; k + 3 < n; k++) {
        struct fixed t = mul(add(in[k], in[k + 1]), fx(n - k));
        struct fixed u = sub(max2(t, in[k + 2]), mul(in[k + 3], fx(2)));
        total = add(total, mul(u, add(t, fx(1))));
    }
    return total.value;
}

int main(void)
{
    struct fixed in[8];
    struct fixed (*op)(struct fixed, struct fixed) = mul;
    struct pair p = { 4, 5 };
    for (int k = 0; k < 8; k++)
        in[k] = fx(k * 7 - 9);

    printf("mul %lld\n", mul(add(fx(3), fx(4)), sub(fx(10), fx(4))).value);
    printf("spread %lld\n", spread(add(fx(1), fx(1)), 3, fx(4), 5, sub(fx(9), fx(3)), fx(7),
                                   add(fx(4), fx(4)), 9, mul(fx(2), fx(5))));
    printf("narrow %d\n", narrow((struct byte){ 200 }, (struct half){ -3 }, (struct word){ 70000 },
                                 (struct ptr){ "xyz" }, (struct byte){ 9 }));
    printf("variadic %lld\n", sum_fixed(4, fx(1), add(fx(1), fx(1)), in[3], mul(fx(2), fx(2))));
    printf("pair %ld\n", with_pair(add(fx(1), fx(1)), p, sub(fx(9), fx(2))));
    printf("pointer %lld\n", op(add(fx(5), fx(6)), fx(3)).value);
    printf("fold %lld\n", fold(fx(1), 9).value);
    printf("bandwidth %lld\n", bandwidth(in, 8));
    return 0;
}
