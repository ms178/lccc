/* `asm` statements whose operands are all in mask registers: every width
 * of the class "k", early clobbers, ties, and values that are no lvalues.
 * No vector is involved, so the file also runs in 32-bit code
 * (asm_mask_register_operands_i686.c), where 64 bits of mask do not fit a
 * general register and move through memory.
 */
#include <stdio.h>

#define NOINLINE __attribute__((noinline))

static int failures;

static void expect(const char *what, unsigned long long got, unsigned long long want)
{
    if (got != want) {
        printf("FAIL %s: %llx, expected %llx\n", what, got, want);
        failures++;
    }
}

NOINLINE static unsigned short logic16(unsigned short a, unsigned short b, unsigned short c)
{
    unsigned short r;
    __asm__("kandw %2, %1, %0\n\tkxorw %3, %0, %0" : "=&k"(r) : "k"(a), "k"(b), "k"(c));
    return r;
}

NOINLINE static unsigned logic32(unsigned a, unsigned b)
{
    unsigned r;
    __asm__("kandnd %2, %1, %0" : "=k"(r) : "k"(a), "k"(b));
    return r;
}

NOINLINE static unsigned long long logic64(unsigned long long a, unsigned long long b)
{
    unsigned long long r;
    __asm__("kxorq %2, %1, %0" : "=k"(r) : "k"(a), "k"(b));
    return r;
}

/* Sixty-four bits that are no lvalue. */
NOINLINE static unsigned long long shifted64(unsigned long long a, int n)
{
    unsigned long long r;
    __asm__("knotq %1, %0" : "=k"(r) : "k"(a << n));
    return r;
}

/* Eight bits out of a wider value. */
NOINLINE static unsigned char low8(int x)
{
    unsigned char r;
    __asm__("knotw %1, %0" : "=k"(r) : "k"((unsigned char)x));
    return r;
}

/* Read and written; tied to the output; a pointee. */
NOINLINE static void in_place(unsigned short *m, unsigned long long *wide)
{
    __asm__("kshiftrw $2, %0, %0" : "+k"(*m));
    __asm__("kshiftlq $9, %0, %0" : "+k"(*wide));
}

NOINLINE static unsigned tied32(unsigned a, unsigned b)
{
    unsigned r;
    __asm__("kord %2, %0, %0" : "=k"(r) : "0"(a), "k"(b));
    return r;
}

/* Seven operands: every mask register that can be a write mask. */
NOINLINE static unsigned short all_seven(unsigned short a, unsigned short b, unsigned short c)
{
    unsigned short r, s, t, u;
    __asm__("kandw %5, %4, %0\n\tkorw %6, %4, %1\n\tkxorw %6, %5, %2\n\tkandnw %6, %4, %3"
            : "=&k"(r), "=&k"(s), "=&k"(t), "=&k"(u) : "k"(a), "k"(b), "k"(c));
    return (unsigned short)(r ^ (s << 1) ^ (t << 2) ^ (u << 3));
}

int main(void)
{
    if (!__builtin_cpu_supports("avx512f") || !__builtin_cpu_supports("avx512bw")) {
        puts("asm mask operands: no AVX-512F and BW on this machine");
        return 0;
    }

    expect("logic16", logic16(0xF0F0, 0x3C3C, 0x00FF), (0xF0F0 & 0x3C3C) ^ 0x00FF);
    expect("logic32", logic32(0x12345678u, 0xFFFF0F0Fu), ~0x12345678u & 0xFFFF0F0Fu);
    expect("logic64", logic64(0x0123456789ABCDEFULL, 0xFEDCBA9876543210ULL),
           0x0123456789ABCDEFULL ^ 0xFEDCBA9876543210ULL);
    expect("shifted64", shifted64(0x00000000F00F1234ULL, 20), ~(0x00000000F00F1234ULL << 20));
    expect("low8", low8(0x7B5A), (unsigned char)~0x5A);

    {
        unsigned short m = 0x8421;
        unsigned long long wide = 0x8000000180000001ULL;
        in_place(&m, &wide);
        expect("in_place 16", m, 0x8421 >> 2);
        expect("in_place 64", wide, 0x8000000180000001ULL << 9);
    }

    expect("tied32", tied32(0x10203040u, 0x01020304u), 0x11223344u);

    {
        unsigned short a = 0x0FF0, b = 0x3333, c = 0x5555;
        unsigned short r = a & b, s = a | c, t = b ^ c, u = (unsigned short)(~a & c);
        expect("all_seven", all_seven(a, b, c), (unsigned short)(r ^ (s << 1) ^ (t << 2) ^ (u << 3)));
    }

    if (failures == 0)
        puts("asm mask operands: PASS");
    return failures != 0;
}
