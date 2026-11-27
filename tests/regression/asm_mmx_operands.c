/* `asm` operands in MMX registers: the class "y".
 *
 * lccc gave such an operand a general register, and the assembler refused
 * `paddd %rsi, %rdx`.  GCC puts 64 bits of integer, a 64-bit vector and 32
 * bits of integer there.  The expected values are computed in C.
 */
#include <mmintrin.h>
#include <stdio.h>
#include <string.h>

#define NOINLINE __attribute__((noinline))

static int failures;

static void expect(const char *what, unsigned long long got, unsigned long long want)
{
    if (got != want) {
        printf("FAIL %s: %llx, expected %llx\n", what, got, want);
        failures++;
    }
}

static __m64 from_bits(unsigned long long bits)
{
    __m64 v;
    memcpy(&v, &bits, sizeof v);
    return v;
}

static unsigned long long to_bits(__m64 v)
{
    unsigned long long bits;
    memcpy(&bits, &v, sizeof bits);
    return bits;
}

/* Two inputs and an output written before the last input is read. */
NOINLINE static unsigned long long add_dwords(unsigned long long a, unsigned long long b)
{
    __m64 va = from_bits(a), vb = from_bits(b), r;
    __asm__("movq %1, %0\n\tpaddd %2, %0" : "=&y"(r) : "y"(va), "y"(vb));
    unsigned long long bits = to_bits(r);
    __asm__("emms");
    return bits;
}

/* An input tied to the output. */
NOINLINE static unsigned long long sub_words(unsigned long long a, unsigned long long b)
{
    __m64 va = from_bits(a), vb = from_bits(b), r;
    __asm__("psubw %2, %0" : "=y"(r) : "0"(va), "y"(vb));
    unsigned long long bits = to_bits(r);
    __asm__("emms");
    return bits;
}

/* Sixty-four bits of integer, read and written. */
NOINLINE static unsigned long long shift_quad(unsigned long long acc, unsigned long long by)
{
    __asm__("psllq %1, %0" : "+y"(acc) : "y"(by));
    __asm__("emms");
    return acc;
}

/* Thirty-two bits of integer. */
NOINLINE static unsigned shift_dword(unsigned a)
{
    unsigned r;
    __asm__("movq %1, %0\n\tpslld $4, %0" : "=&y"(r) : "y"(a));
    __asm__("emms");
    return r;
}

/* An operand that is no lvalue. */
NOINLINE static unsigned long long or_sum(unsigned long long a, unsigned long long b)
{
    __m64 va = from_bits(a), vb = from_bits(b), r;
    __asm__("movq %1, %0\n\tpor %2, %0" : "=&y"(r) : "y"(_mm_add_pi32(va, vb)), "y"(vb));
    unsigned long long bits = to_bits(r);
    __asm__("emms");
    return bits;
}

static unsigned long long dwords(unsigned lo, unsigned hi)
{
    return (unsigned long long)hi << 32 | lo;
}

int main(void)
{
    if (!__builtin_cpu_supports("mmx")) {
        puts("asm MMX operands: no MMX on this machine");
        return 0;
    }

    unsigned long long a = 0x0123456789ABCDEFULL, b = 0xF0E0D0C0B0A09080ULL;

    expect("add_dwords", add_dwords(a, b),
           dwords((unsigned)a + (unsigned)b, (unsigned)(a >> 32) + (unsigned)(b >> 32)));

    {
        unsigned long long want = 0;
        for (int i = 0; i < 4; i++) {
            unsigned short x = (unsigned short)(a >> (16 * i)), y = (unsigned short)(b >> (16 * i));
            want |= (unsigned long long)(unsigned short)(x - y) << (16 * i);
        }
        expect("sub_words", sub_words(a, b), want);
    }

    expect("shift_quad", shift_quad(a, 12), a << 12);
    expect("shift_dword", shift_dword(0x0ABCDEF1u), 0x0ABCDEF1u << 4);
    expect("or_sum", or_sum(a, b),
           dwords((unsigned)a + (unsigned)b, (unsigned)(a >> 32) + (unsigned)(b >> 32)) | b);

    if (failures == 0)
        puts("asm MMX operands: PASS");
    return failures != 0;
}
