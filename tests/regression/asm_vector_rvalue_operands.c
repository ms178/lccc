/* A 128-bit or 64-bit vector that is no lvalue as a register operand of an
 * `asm` statement.
 *
 * To the IR a vector is the address of its bytes; the register wants the
 * bytes.  lccc left the load to the back end, which read an address that
 * was no plain variable's as if it were the value: `"x"(c ? a : b)` gave
 * the bits of a pointer at -O0, and a 64-bit vector in any register class
 * (`"r"(v)`, `"y"(v)`) was the address of the variable at every level.
 */
#include <stdio.h>
#include <string.h>

typedef int v4si __attribute__((vector_size(16)));
typedef short v4hi __attribute__((vector_size(8)));

#define NOINLINE __attribute__((noinline))

static int failures;

static void expect4(const char *what, v4si got, int a, int b, int c, int d)
{
    v4si want = { a, b, c, d };
    if (memcmp(&got, &want, sizeof want) != 0) {
        printf("FAIL %s: %d %d %d %d\n", what, got[0], got[1], got[2], got[3]);
        failures++;
    }
}

static void expect_bits(const char *what, unsigned long long got, unsigned long long want)
{
    if (got != want) {
        printf("FAIL %s: %llx, expected %llx\n", what, got, want);
        failures++;
    }
}

NOINLINE static v4si twice(v4si v) { return v + v; }

/* Each doubles its operand: `pslld $1` on a register tied to the output. */
#define DOUBLED(expr) __asm__("pslld $1, %0" : "=x"(r) : "0"(expr))

NOINLINE static v4si sum(v4si a, v4si b) { v4si r; DOUBLED(a + b); return r; }
NOINLINE static v4si chosen(v4si a, v4si b, int c) { v4si r; DOUBLED(c ? a : b); return r; }
NOINLINE static v4si literal(void) { v4si r; DOUBLED(((v4si){ 1, 2, 3, 4 })); return r; }
NOINLINE static v4si block(v4si a, v4si b) { v4si r; DOUBLED(({ v4si t = a - b; t; })); return r; }
NOINLINE static v4si second(v4si a, v4si b) { v4si r; DOUBLED((a, b)); return r; }
NOINLINE static v4si called(v4si a) { v4si r; DOUBLED(twice(a)); return r; }
NOINLINE static v4si chosen_call(v4si a, v4si b, int c) { v4si r; DOUBLED(c ? twice(a) : b); return r; }
NOINLINE static v4si member(const struct { int tag; v4si v; } *s) { v4si r; DOUBLED(s->v); return r; }

/* Sixty-four bits of vector in a general register. */
NOINLINE static unsigned long long bits_of(v4hi v)
{
    unsigned long long r;
    __asm__("movq %1, %0" : "=r"(r) : "r"(v));
    return r;
}

NOINLINE static unsigned long long bits_of_sum(v4hi a, v4hi b)
{
    unsigned long long r;
    __asm__("movq %1, %0" : "=r"(r) : "r"(a + b));
    return r;
}

NOINLINE static unsigned long long bits_of_choice(v4hi a, v4hi b, int c)
{
    unsigned long long r;
    __asm__("movq %1, %0" : "=r"(r) : "r"(c ? a : b));
    return r;
}

/* Read and written in a general register. */
NOINLINE static v4hi shifted(v4hi v)
{
    __asm__("shlq $8, %0" : "+r"(v));
    return v;
}

int main(void)
{
    v4si a = { 1, 2, 3, 4 }, b = { 10, 20, 30, 40 };
    struct { int tag; v4si v; } s = { 7, { 5, 6, 7, 8 } };

    expect4("sum", sum(a, b), 22, 44, 66, 88);
    expect4("chosen 1", chosen(a, b, 1), 2, 4, 6, 8);
    expect4("chosen 0", chosen(a, b, 0), 20, 40, 60, 80);
    expect4("literal", literal(), 2, 4, 6, 8);
    expect4("block", block(a, b), -18, -36, -54, -72);
    expect4("second", second(a, b), 20, 40, 60, 80);
    expect4("called", called(a), 4, 8, 12, 16);
    expect4("chosen_call 1", chosen_call(a, b, 1), 4, 8, 12, 16);
    expect4("chosen_call 0", chosen_call(a, b, 0), 20, 40, 60, 80);
    expect4("member", member((void *)&s), 10, 12, 14, 16);

    v4hi h = { 0x1111, 0x2222, 0x3333, 0x4444 }, k = { 1, 2, 3, 4 };
    expect_bits("bits_of", bits_of(h), 0x4444333322221111ULL);
    expect_bits("bits_of_sum", bits_of_sum(h, k), 0x4448333622241112ULL);
    expect_bits("bits_of_choice 1", bits_of_choice(h, k, 1), 0x4444333322221111ULL);
    expect_bits("bits_of_choice 0", bits_of_choice(h, k, 0), 0x0004000300020001ULL);
    {
        v4hi moved = shifted(h);
        unsigned long long bits;
        memcpy(&bits, &moved, sizeof bits);
        expect_bits("shifted", bits, 0x4444333322221111ULL << 8);
    }

    if (failures == 0)
        puts("asm vector rvalue operands: PASS");
    return failures != 0;
}
