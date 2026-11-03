/* Unsigned division and remainder by a power of two, including 1.
 *
 * AArch64 lowers `x % 2^k` to `and x, x, #(2^k - 1)`; for 1 the mask is 0,
 * which no logical immediate encodes (`and w0, w0, #0` was rejected by the
 * assembler at -O1 for a vector remainder by { 1, 4, 2, 8 } lowered lane by
 * lane: gcc.c-torture execute/pr53645.c).  The remainder by 1 is the
 * constant 0.
 *
 * Self-checking: exit status 0 is success; abort() otherwise. */
extern void abort(void);

typedef unsigned int UV __attribute__((vector_size(16)));
typedef unsigned long long ULV __attribute__((vector_size(16)));

__attribute__((noinline)) static void urem_1428(UV *x, const UV *y) { *x = *y % (UV){1, 4, 2, 8}; }
__attribute__((noinline)) static void udiv_1428(UV *x, const UV *y) { *x = *y / (UV){1, 4, 2, 8}; }
__attribute__((noinline)) static void urem_1_64(ULV *x, const ULV *y) { *x = *y % (ULV){1, 16}; }
__attribute__((noinline)) static unsigned rem_one(unsigned x) { return x % 1u; }
__attribute__((noinline)) static unsigned long long rem_one_64(unsigned long long x) { return x % 1ull; }

static volatile unsigned inputs[] = {0, 1, 7, 73, 65531, 0x7fffffff, 0x80000000, 0xffffffff};

int main(void)
{
    unsigned n = sizeof(inputs) / sizeof(inputs[0]);
    for (unsigned i = 0; i + 3 < n; i++) {
        UV y = {inputs[i], inputs[i + 1], inputs[i + 2], inputs[i + 3]}, r, q;
        urem_1428(&r, &y);
        udiv_1428(&q, &y);
        if (r[0] != 0 || r[1] != y[1] % 4 || r[2] != y[2] % 2 || r[3] != y[3] % 8)
            abort();
        if (q[0] != y[0] || q[1] != y[1] / 4 || q[2] != y[2] / 2 || q[3] != y[3] / 8)
            abort();
        ULV z = {(unsigned long long)inputs[i] << 32 | inputs[i + 1], inputs[i + 2]}, s;
        urem_1_64(&s, &z);
        if (s[0] != 0 || s[1] != (z[1] & 15))
            abort();
    }
    for (unsigned i = 0; i < n; i++)
        if (rem_one(inputs[i]) != 0 || rem_one_64((unsigned long long)inputs[i] << 31) != 0)
            abort();
    return 0;
}
