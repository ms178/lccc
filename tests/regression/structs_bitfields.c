/* struct/union layout, bitfield semantics, alignment,
 * by-value passing, return-in-memory. */
#include <stdio.h>
#include <stddef.h>
#include <stdint.h>

struct S {
    char c;
    int i;
    short s;
};
struct Bit {
    unsigned a : 3;
    unsigned b : 5;
    unsigned c : 8;
    signed   d : 4;
    unsigned : 0;        /* force alignment to next unit */
    unsigned e : 1;
};
union U {
    uint32_t u;
    uint8_t b[4];
    struct { uint8_t lo; uint8_t hi; } w;
};

struct Big { uint64_t v[4]; };   /* 32 bytes -> return in memory */

static struct Big make_big(uint64_t a, uint64_t b, uint64_t c, uint64_t d) {
    struct Big r = {{a, b, c, d}};
    return r;
}
static struct S mkS(char c, int i, short s) {
    struct S r = {c, i, s};
    return r;
}
static int sumS(struct S x) { return x.c + x.i + x.s; }

int main(void) {
    /* layout: offsets per SysV x86-64 (int align 4, short align 2) */
    if (offsetof(struct S, c) != 0) return 1;
    if (offsetof(struct S, i) != 4) return 2;
    if (offsetof(struct S, s) != 8) return 3;
    if (sizeof(struct S) != 12) return 4;

    /* bitfields */
    struct Bit bf;
    bf.a = 5; bf.b = 17; bf.c = 200; bf.d = -3; bf.e = 1;
    if (bf.a != 5) return 5;
    if (bf.b != 17) return 6;
    if (bf.c != 200) return 7;
    if (bf.d != -3) return 8;          /* signed 4-bit field */
    if (bf.e != 1) return 9;
    bf.a = 7; if (bf.a != 7) return 10;
    bf.a = 8; if (bf.a != 0) return 11; /* overflow wraps in 3 bits */
    if (sizeof(bf) != 8) return 12;     /* two 32-bit units */

    /* union */
    union U uu;
    uu.u = 0x01020304u;
    if (uu.b[0] != 0x04 || uu.b[3] != 0x01) return 13;  /* little-endian */
    uu.w.lo = 0xAA; uu.w.hi = 0xBB;  /* only bytes 0-1 change */
    if (uu.u != 0x0102BBAAu) return 14;

    /* by-value + return-in-memory */
    struct S s = mkS(1, 2, 3);
    if (sumS(s) != 6) return 15;
    struct Big bg = make_big(1, 2, 3, 4);
    if (bg.v[0] != 1 || bg.v[1] != 2 || bg.v[2] != 3 || bg.v[3] != 4) return 16;

    /* nested */
    struct { struct S s; int x; } n;
    n.s = s; n.x = 9;
    if (n.s.i != 2 || n.x != 9) return 17;
    if (sizeof(n) != 16) return 18;

    /* array of structs */
    struct S arr[3] = {{1,2,3},{4,5,6},{7,8,9}};
    int tot = 0;
    for (int i = 0; i < 3; i++) tot += arr[i].i;
    if (tot != 15) return 19;

    /* packed-ish via explicit layout */
    struct { char a; char b; int c; } __attribute__((packed)) pk;
    if (offsetof(typeof(pk), c) != 2) return 20;

    printf("OK structs_bitfields\n");
    return 0;
}
