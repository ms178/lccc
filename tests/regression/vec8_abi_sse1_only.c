/* x86-64 passes and returns 8-byte vectors (`__m64`, `float x2`, `int x2`)
 * and SSE-class eightbytes of small structs in XMM registers. Under
 * `-mno-sse2 -msse` there is no MOVQ/MOVSD between XMM and memory: lccc
 * stored such a parameter or call result with them and its SSE2 guard
 * rejected `void f(__m64 *p, __m64 a) { _mm_stream_pi(p, a); }`, which GCC
 * compiles with MOVLPS. Every shape below goes through a non-inlined call,
 * as argument, result and stack argument, and is compared against GCC. */
#include <stdio.h>
#include <string.h>
#include <xmmintrin.h>

typedef float F2 __attribute__((vector_size(8)));
typedef int I2 __attribute__((vector_size(8)));
struct SM { __m64 m; };
struct SFL { F2 f; long l; };
struct SLF { long l; F2 f; };
struct SFF { F2 a, b; };

static unsigned long long h = 1469598103934665603ULL;
static void mix(const void *p, unsigned n)
{
    const unsigned char *b = p;
    for (unsigned i = 0; i < n; ++i)
        h = (h ^ b[i]) * 1099511628211ULL;
}
#define MIX(v) do { __typeof__(v) t_ = (v); mix(&t_, sizeof t_); } while (0)
#define NI __attribute__((noinline))

NI static __m64 m_add(__m64 a, __m64 b) { return _mm_add_pi16(a, b); }
NI static void m_stream(__m64 *p, __m64 a) { _mm_stream_pi(p, a); }
NI static __m64 m_load(const __m64 *p) { return *p; }
NI static I2 i_swap(I2 a) { return (I2){a[1], a[0]}; }
NI static F2 f_pass(F2 a, F2 b) { F2 r; memcpy(&r, &b, 4); memcpy((char *)&r + 4, (char *)&a + 4, 4); return r; }
NI static struct SM sm_xor(struct SM a, struct SM b) { a.m = _mm_xor_si64(a.m, b.m); return a; }
NI static struct SFL sfl(struct SFL a) { a.l ^= 0x5a5a; F2 t = a.f; memcpy(&a.f, (char *)&t + 4, 4); memcpy((char *)&a.f + 4, &t, 4); return a; }
NI static struct SLF slf(struct SLF a) { a.l += 7; return a; }
NI static struct SFF sff(struct SFF a) { struct SFF r = {a.b, a.a}; return r; }
/* Nine 8-byte vectors: the last one travels on the stack. */
NI static __m64 m_many(__m64 a, __m64 b, __m64 c, __m64 d, __m64 e, __m64 f, __m64 g, __m64 i, __m64 j)
{
    return _mm_xor_si64(_mm_xor_si64(_mm_xor_si64(a, b), _mm_xor_si64(c, d)),
                        _mm_xor_si64(_mm_xor_si64(e, f), _mm_xor_si64(_mm_xor_si64(g, i), _mm_slli_si64(j, 3))));
}

int main(void)
{
    __m64 (*volatile add)(__m64, __m64) = m_add;
    for (volatile int k = 0; k < 3; ++k) {
        __m64 a = _mm_set_pi16((short)(k * 7 - 3), 2, -32768, (short)k), b = _mm_set_pi32(-k, 0x7fff0001);
        MIX(m_add(a, b));
        MIX(add(b, a));
        __m64 slot;
        m_stream(&slot, m_add(a, a));
        _mm_sfence();
        MIX(slot);
        MIX(m_load(&slot));
        I2 iv = {k, -k - 1};
        MIX(i_swap(iv));
        F2 fa, fb;
        unsigned u[4] = {0x3f800000u + (unsigned)k, 0xc0000000u, 0x7fc00001u, 0x00000001u};
        memcpy(&fa, u, 8);
        memcpy(&fb, u + 2, 8);
        MIX(f_pass(fa, fb));
        struct SM sa = {a}, sb = {b};
        MIX(sm_xor(sa, sb));
        struct SFL x = {fa, -k};
        MIX(sfl(x));
        struct SLF y = {k * 3, fb};
        MIX(slf(y));
        struct SFF z = {fa, fb};
        MIX(sff(z));
        MIX(m_many(a, b, a, b, b, a, a, b, _mm_set_pi8(1, 2, 3, 4, 5, 6, 7, (char)k)));
    }
    _mm_empty();
    printf("m64 abi sse1 %016llx\n", h);
    return 0;
}
