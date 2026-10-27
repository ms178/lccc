/* `__builtin_shufflevector` with any lane count (GCC 12+, Clang): the result
 * has the operands' element type and one lane per index, and the operands
 * may differ in lane count. lccc used to accept only 4 x 32-bit operands with
 * four indices and rejected everything else. Covers narrowing, widening,
 * mixed-width operands, 1/2/4/8/16/32-lane results, 1/2/4/8-byte elements,
 * float and double lanes, results of at most 8 bytes (x86-64 packed register
 * carriers) inside conditionals, as arguments and as returns, and the
 * retained 4 x 32-bit path. Every result lane is compared against GCC; -1
 * ("any lane") appears only in lanes that are not observed. `typeof` of
 * either shuffle builtin used to fall back to `int` with only a warning. */
#include <stdio.h>
#include <string.h>

typedef signed char v16qi __attribute__((vector_size(16)));
typedef signed char v8qi __attribute__((vector_size(8)));
typedef signed char v32qi __attribute__((vector_size(32)));
typedef short v8hi __attribute__((vector_size(16)));
typedef short v4hi __attribute__((vector_size(8)));
typedef short v2hi __attribute__((vector_size(4)));
typedef int v4si __attribute__((vector_size(16)));
typedef int v8si __attribute__((vector_size(32)));
typedef int v2si __attribute__((vector_size(8)));
typedef int v1si __attribute__((vector_size(4)));
typedef long long v2di __attribute__((vector_size(16)));
typedef long long v4di __attribute__((vector_size(32)));
typedef float v4sf __attribute__((vector_size(16)));
typedef float v2sf __attribute__((vector_size(8)));
typedef double v2df __attribute__((vector_size(16)));
typedef double v4df __attribute__((vector_size(32)));

static unsigned long long hash = 1469598103934665603ULL;
static void mixb(const void *p, unsigned n)
{
    const unsigned char *b = p;
    for (unsigned i = 0; i < n; ++i)
        hash = (hash ^ b[i]) * 1099511628211ULL;
}
#define MIX(v) do { __typeof__(v) t_ = (v); mixb(&t_, sizeof t_); } while (0)

__attribute__((noinline)) static v2si narrow8(v4si a, v4si b) { return __builtin_shufflevector(a, b, 7, 0); }
__attribute__((noinline)) static v4si widen_mixed(v2si a, v4si b) { return __builtin_shufflevector(a, b, 5, 0, 2, 1); }
__attribute__((noinline)) static v2hi narrow4(v8hi a) { return __builtin_shufflevector(a, a, 7, 3); }
__attribute__((noinline)) static long long take(v2si v) { long long x; memcpy(&x, &v, 8); return x; }

int main(void)
{
    for (volatile int round = 0; round < 3; ++round) {
        int k = round * 37;
        v8si a8 = {1 + k, -2, 3, -4, 5, -6, 7, -8 - k};
        v4si c4 = {-2, -4 - k, -6, -8};
        _Static_assert(sizeof(__typeof__(__builtin_shuffle(c4, c4))) == 16, "shuffle typeof");
        _Static_assert(sizeof(__typeof__(__builtin_shufflevector(c4, c4, 0, 1))) == 8, "typeof");
        _Static_assert(sizeof(__typeof__(__builtin_shufflevector(a8, c4, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10,
                                                                  11, 0, 1, 2, 3))) == 64, "wide");
        /* The GCC manual's own example, both directions. */
        v4si b4 = __builtin_shufflevector(a8, a8, 0, 2, 4, 6);
        v8si d8 = __builtin_shufflevector(c4, b4, 4, 0, 5, 1, 6, 2, 7, 3);
        MIX(b4); MIX(d8);
        /* Bytes: full reverse, 32-lane widen, 8-byte narrow. */
        v16qi q = {0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, (signed char)(15 + k)};
        v16qi qr = __builtin_shufflevector(q, q, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0);
        v32qi qw = __builtin_shufflevector(q, qr, 0, 16, 1, 17, 2, 18, 3, 19, 4, 20, 5, 21, 6, 22, 7, 23,
                                           8, 24, 9, 25, 10, 26, 11, 27, 12, 28, 13, 29, 14, 30, 15, 31);
        v8qi qn = __builtin_shufflevector(q, q, 1, 3, 5, 7, 9, 11, 13, 15);
        MIX(qr); MIX(qw); MIX(qn);
        /* Shorts: interleave with a narrower operand, 4-byte result. */
        v8hi h = {10, 11, 12, 13, 14, 15, 16, (short)(17 + k)};
        v4hi h4 = {-1, -2, -3, (short)(-4 - k)};
        v8hi hi = __builtin_shufflevector(h, h4, 0, 8, 1, 9, 2, 10, 3, 11);
        v4hi hn = __builtin_shufflevector(h4, h, 3, 4, 0, 11);
        MIX(hi); MIX(hn); MIX(narrow4(h));
        /* Ints: 1-lane, 8-byte packed results in every consumer context. */
        v1si one = __builtin_shufflevector(c4, c4, 3);
        v2si n2 = narrow8(c4, b4);
        v2si sel = round == 1 ? n2 : __builtin_shufflevector(c4, b4, 1, 6);
        MIX(one); MIX(n2); MIX(sel);
        MIX(take(__builtin_shufflevector(b4, c4, 2, 5)));
        MIX(widen_mixed(n2, c4));
        MIX((long long)__builtin_shufflevector(c4, c4, 0, 3));
        MIX(__builtin_shufflevector(c4, b4, 1, 7)[1]);
        v2si acc = n2;
        acc += __builtin_shufflevector(b4, b4, 3, 2);
        MIX(acc);
        /* 64-bit lanes and floating lanes. */
        v2di l = {0x0123456789abcdefLL + k, -7};
        v4di lw = __builtin_shufflevector(l, l, 1, 0, 0, 1);
        v4sf f = {1.5f, -2.25f, 3.0f + k, 1e30f};
        v2sf fn = __builtin_shufflevector(f, f, 3, 1);
        v4sf fs = __builtin_shufflevector(f, f, 2, 2, 0, 1);
        v2df dd = {0.5, -1.0 - k};
        v4df dw = __builtin_shufflevector(dd, dd, 1, 0, 1, 0);
        MIX(lw); MIX(fn); MIX(fs); MIX(dw);
        /* Retained 4 x 32-bit path: single-source PSHUFD and mixed gather. */
        MIX(__builtin_shufflevector(c4, b4, 3, 2, 1, 0));
        MIX(__builtin_shufflevector(c4, b4, 0, 5, 2, 7));
        MIX(__builtin_shufflevector(c4, b4, 0, 5, 2, -1)[2]);
    }
    printf("shufflevector any width %016llx\n", hash);
    return 0;
}
