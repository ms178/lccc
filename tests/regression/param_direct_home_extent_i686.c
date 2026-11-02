/* i686 directly-homed stack parameters: every byte of the parameter's
 * extent must address the caller's argument slot, through every frame
 * operand former (plain, interior offset, register-indexed SIB, memcpy
 * endpoint), for reads and writes, at every parameter width.
 *
 * Before the fix only the exact home offset was redirected: interior and
 * indexed accesses read the never-written frame slot (gcc-16 -m32 prints
 * "2114 44 -16" for the first line, the broken build printed "0 44 -1").
 * Wrappers re-run this file at -O1 / -Os and with/without a frame pointer. */
#include <stdio.h>
#include <string.h>

#define NI __attribute__((noinline))
static int fails;
#define CHECK(c) do { if (!(c)) { printf("FAIL line %d\n", __LINE__); fails++; } } while (0)

NI int f(unsigned a, unsigned b) {
    unsigned char *p = (unsigned char *)&a, *q = (unsigned char *)&b;
    return p[1] * 1000 + q[2] * 10 + p[3];
}
NI int g(unsigned long long a) { unsigned char *p = (unsigned char *)&a; return p[5] * 7 + p[1]; }
NI int h(unsigned a, int i) { unsigned char *p = (unsigned char *)&a; return p[i] - p[i + 1]; }

/* Narrow parameters: the extent is the alloca's data size. */
NI int narrow(signed char c, short s, int i) {
    unsigned char *ps = (unsigned char *)&s, *pi = (unsigned char *)&i;
    return c + ps[0] + ps[1] * 3 + pi[2] * 5;
}
NI int narrow_idx(short s, int k) { return ((unsigned char *)&s)[k]; }

/* Writes through interior pointers must land in the home the whole-value
 * read uses. */
NI unsigned poke(unsigned a, int k, unsigned char v) { ((unsigned char *)&a)[k] = v; return a; }
NI unsigned poke1(unsigned a) { ((unsigned char *)&a)[1] = 0x77; return a; }
/* Character lvalues only: they may alias any object (C11 6.5p7). */
NI unsigned long long poke64(unsigned long long a, int k) {
    unsigned char *p = (unsigned char *)&a;
    p[2 * k] = 0xef;
    p[2 * k + 1] = 0xbe;
    return a;
}

/* memcpy endpoints in a parameter home, both directions and every window. */
NI unsigned from_home(unsigned a) { unsigned char b[4]; memcpy(b, &a, 4); return b[0] | b[1] << 8 | b[2] << 16 | (unsigned)b[3] << 24; }
NI unsigned into_home(unsigned a, const unsigned char *src) { memcpy(&a, src, 3); return a; }
NI unsigned long long home64(unsigned long long a) { unsigned long long r; memcpy(&r, &a, sizeof r); return r ^ 0x0102030405060708ull; }
NI unsigned mid_home(unsigned a) { unsigned char t[2]; memcpy(t, (char *)&a + 1, 2); return t[0] * 256u + t[1]; }

/* The address escapes: the callee reads the caller's argument slot. */
NI int peek(const unsigned char *p, int k) { return p[k]; }
NI int escape(unsigned a) { return peek((unsigned char *)&a, 2) * 100 + peek((unsigned char *)&a, 3); }

/* Float parameter: same rule, different value class. */
NI int fbytes(float x) { unsigned char *p = (unsigned char *)&x; return p[3] * 256 + p[2]; }

int main(void) {
    printf("%d %d %d\n", f(0x04030201u, 0x0c0b0a09u), g(0x0807060504030201ull), h(0x40302010u, 1));
    CHECK(f(0x04030201u, 0x0c0b0a09u) == 2114);
    CHECK(g(0x0807060504030201ull) == 44);
    CHECK(h(0x40302010u, 1) == -16 && h(0x40302010u, 0) == -16 && h(0x40302010u, 2) == -16);
    CHECK(narrow(-3, 0x0201, 0x00090000) == -3 + 1 + 6 + 45);
    CHECK(narrow_idx(0x5a3c, 0) == 0x3c && narrow_idx(0x5a3c, 1) == 0x5a);
    for (int k = 0; k < 4; k++)
        CHECK(poke(0x11223344u, k, 0xee) == ((0x11223344u & ~(0xffu << 8 * k)) | 0xeeu << 8 * k));
    CHECK(poke1(0x11223344u) == 0x11227744u);
    for (int k = 0; k < 4; k++)
        CHECK(poke64(0x1111222233334444ull, k)
              == ((0x1111222233334444ull & ~(0xffffull << 16 * k)) | 0xbeefull << 16 * k));
    CHECK(from_home(0xa1b2c3d4u) == 0xa1b2c3d4u);
    static const unsigned char src[3] = {0x01, 0x02, 0x03};
    CHECK(into_home(0xffffffffu, src) == 0xff030201u);
    CHECK(home64(0x1112131415161718ull) == (0x1112131415161718ull ^ 0x0102030405060708ull));
    CHECK(mid_home(0x44332211u) == 0x2233u);
    CHECK(escape(0x04030201u) == 304);
    CHECK(fbytes(1.0f) == 0x3f80);
    printf("%s %d\n", fails ? "FAILED" : "ok", fails);
    return fails != 0;
}
