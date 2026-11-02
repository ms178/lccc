/* Regression test: `movb $imm, %dil/%sil/%bpl/%spl` must carry a REX prefix.
 *
 * The byte registers %spl, %bpl, %sil and %dil encode as 4..7 -- the SAME
 * register numbers as the legacy high-byte registers %ah, %ch, %dh and %bh.
 * The two sets are distinguished ONLY by the presence of a REX prefix. The
 * B0+r form of `mov imm8, r8` was emitted without REX for these four, so
 *
 *     movb $80, %dil        assembled as        movb $80, %bh
 *
 * which writes bits 8-15 of RBX instead of the low byte of RDI. When the
 * clobbered register held a live pointer the result was a wild store:
 *
 *     leaq 0x54(%rsp), %rbx
 *     movb $80, %bh          <- was meant to be %dil, corrupts the pointer
 *     movb %dil, (%rbx)      <- stores through the corrupted pointer
 *
 * That is exactly the `p.name[0] = 'P'` assignment in the struct_copy
 * benchmark, which SIGSEGV'd on roughly one run in five at -O0. The crash was
 * intermittent because it depended on whether the corrupted address happened
 * to land in a mapped page.
 *
 * The same aliasing applies to r8b-r15b (handled by the REX.B path).
 *
 * Scope note: this is the END-TO-END guard (compiler -> assembler -> run). It
 * checks that byte stores through live pointers produce the right bytes at
 * runtime, but it cannot force the register allocator to pick a REX-only byte
 * register, so it does not by itself reproduce the encoding bug. The exhaustive
 * encoding check lives in tests/asm-diff/byteregs.casefile, which assembles
 * `movb $imm` into all twenty byte registers plus the full byte-ALU/setcc/movzx
 * matrix and diffs the bytes against GNU as. Both are needed: the casefile
 * catches the encoder, this catches everything downstream of it.
 */

#include <string.h>

struct rec {
    char  name[8];
    int   id;
    double val;
};

/* Mirrors make_particle: build a struct in a local, set a byte field through
 * a pointer, and return it by value. */
__attribute__((noinline))
static struct rec build(int i)
{
    struct rec r;
    memset(&r, 0, sizeof r);
    r.val = (double)i * 0.5;
    r.id = i;
    r.name[0] = 'P';                 /* the byte store that was miscompiled */
    r.name[1] = (char)('0' + i % 10);
    r.name[2] = '\0';
    return r;
}

/* Force many simultaneously-live byte values so the register allocator has to
 * use the REX-only byte registers rather than just al/cl/dl/bl. */
__attribute__((noinline))
static unsigned long many_bytes(unsigned char seed, unsigned char *out)
{
    unsigned char a = (unsigned char)(seed + 1);
    unsigned char b = (unsigned char)(seed + 2);
    unsigned char c = (unsigned char)(seed + 3);
    unsigned char d = (unsigned char)(seed + 4);
    unsigned char e = (unsigned char)(seed + 5);
    unsigned char f = (unsigned char)(seed + 6);
    unsigned char g = (unsigned char)(seed + 7);
    unsigned char h = (unsigned char)(seed + 8);
    /* keep every value live across the stores */
    out[0] = a; out[1] = b; out[2] = c; out[3] = d;
    out[4] = e; out[5] = f; out[6] = g; out[7] = h;
    return (unsigned long)a + b + c + d + e + f + g + h;
}

/* A pointer that must survive a neighbouring byte-immediate store. If the
 * store lands in the high half of the pointer register, this writes wild. */
__attribute__((noinline))
static int store_via_pointer(char *buf, int n)
{
    char *p = buf;
    for (int i = 0; i < n; i++) {
        *p = 'P';                    /* byte immediate through a live pointer */
        p++;
        *p = (char)('0' + (i % 10));
        p++;
    }
    return (int)(p - buf);
}

int main(void)
{
    for (int i = 0; i < 10; i++) {
        struct rec r = build(i);
        if (r.name[0] != 'P') return 1;
        if (r.name[1] != (char)('0' + i % 10)) return 2;
        if (r.name[2] != '\0') return 3;
        if (r.id != i) return 4;
        if (r.val != (double)i * 0.5) return 5;
    }

    {
        unsigned char out[8];
        unsigned long sum = many_bytes(100, out);
        if (sum != (unsigned long)(101 + 102 + 103 + 104 + 105 + 106 + 107 + 108))
            return 6;
        for (int i = 0; i < 8; i++)
            if (out[i] != (unsigned char)(101 + i)) return 7;
    }

    {
        char buf[64];
        memset(buf, 0, sizeof buf);
        if (store_via_pointer(buf, 16) != 32) return 8;
        for (int i = 0; i < 16; i++) {
            if (buf[2 * i] != 'P') return 9;
            if (buf[2 * i + 1] != (char)('0' + (i % 10))) return 10;
        }
    }

    return 0;
}
