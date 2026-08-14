/* Regression test: small inline struct copies must not leave the upper half of
 * a ymm register dirty.
 *
 * A 32-byte chunk copied as one `vmovdqu %ymm0` saves exactly ONE instruction
 * over two 16-byte moves. It also leaves the upper 128 bits of ymm0 live, and
 * every legacy-SSE instruction executed afterwards -- including the ordinary
 * scalar FP this backend emits (movsd / mulsd / addsd / subsd) -- then pays an
 * AVX->SSE state transition of roughly 70 cycles on Intel. A loop that copies
 * a struct and then does floating-point work on it crosses the boundary twice
 * per iteration.
 *
 * Measured on tests/benchmark/programs/struct_copy.c (48-byte struct, three
 * copies per iteration, 2,000,000 iterations, -O2, median of 7):
 *
 *     ymm chunk + legacy SSE tail   5287 ms
 *     ymm chunk + VEX tail          2516 ms
 *     no ymm for small copies        113 ms      <- 46x faster
 *     gcc -O2                         21 ms
 *
 * This test is a CORRECTNESS guard for the copies themselves: the sizes below
 * cover every arm of the inline memcpy expansion (64/48/32/16/8/4/2/1 bytes
 * and the awkward in-between sizes), mixed with scalar FP so that a future
 * change reintroducing a dirty ymm still has to keep the data right. The
 * performance property is asserted separately by the benchmark suite; here we
 * make sure no expansion arm ever corrupts or drops bytes.
 */

#include <string.h>

typedef struct { double x, y, z; int id; char name[20]; } P48;   /* 48 bytes */
typedef struct { double a[8]; } S64;                              /* 64 bytes */
typedef struct { double a, b; int i; } S24;                       /* 24 bytes */
typedef struct { char b[33]; } S33;                               /* 33 bytes */
typedef struct { char b[17]; } S17;                               /* 17 bytes */
typedef struct { char b[7];  } S7;                                /*  7 bytes */

__attribute__((noinline)) static P48 mk48(int i)
{
    P48 p;
    p.x = (double)i * 0.5; p.y = (double)i * 0.25; p.z = (double)i * 0.125;
    p.id = i;
    for (int k = 0; k < 20; k++) p.name[k] = (char)('a' + (i + k) % 26);
    return p;                       /* by-value return: an inline 48-byte copy */
}

__attribute__((noinline)) static double use48(P48 a, P48 b)
{
    /* scalar FP right next to the copies: this is what triggers the transition */
    double dx = a.x - b.x, dy = a.y - b.y, dz = a.z - b.z;
    return dx * dx + dy * dy + dz * dz;
}

int main(void)
{
    /* 48-byte structs: copy, pass by value, and verify every field survives. */
    {
        double acc = 0;
        for (int i = 0; i < 64; i++) {
            P48 a = mk48(i);
            P48 b = a;                       /* struct assignment */
            P48 c = mk48(i + 1);
            if (b.id != i) return 1;
            if (b.x != (double)i * 0.5) return 2;
            for (int k = 0; k < 20; k++)
                if (b.name[k] != (char)('a' + (i + k) % 26)) return 3;
            acc += use48(b, c);
        }
        /* each pair differs by exactly one step in x,y,z */
        double one = 0.5 * 0.5 + 0.25 * 0.25 + 0.125 * 0.125;
        double want = one * 64;
        double d = acc - want;
        if (d < -1e-9 || d > 1e-9) return 4;
    }

    /* Every inline-expansion arm, checked byte-exactly. */
    {
        unsigned char src[128], dst[128];
        for (int i = 0; i < 128; i++) src[i] = (unsigned char)(i * 7 + 1);

#define CHECK(TYPE, N)                                                        \
        do {                                                                  \
            TYPE s, d2;                                                       \
            memcpy(&s, src, sizeof(TYPE));                                    \
            d2 = s;                             /* inline struct copy */      \
            memset(dst, 0, sizeof dst);                                       \
            memcpy(dst, &d2, sizeof(TYPE));                                   \
            if (memcmp(dst, src, sizeof(TYPE)) != 0) return (N);              \
        } while (0)

        CHECK(S64, 10);
        CHECK(P48, 11);
        CHECK(S33, 12);
        CHECK(S24, 13);
        CHECK(S17, 14);
        CHECK(S7,  15);
#undef CHECK

        /* raw memcpy of every size 1..80: no arm may over- or under-copy */
        for (unsigned n = 1; n <= 80; n++) {
            memset(dst, 0xAA, sizeof dst);
            memcpy(dst, src, n);
            if (memcmp(dst, src, n) != 0) return 20;
            for (unsigned k = n; k < 80; k++)
                if (dst[k] != 0xAA) return 21;     /* wrote past the end */
        }
    }

    return 0;
}
