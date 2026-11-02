/* x86-64 returns a GNU vector of at most 8 bytes in a register, so lccc
 * lowers such a call to the packed bits, not to the address that every other
 * vector rvalue yields. A conditional, comma or statement expression used to
 * forward those bits unchanged, and every consumer that recognised only a
 * bare call then dereferenced them: `T r = c ? v : f();` segfaulted, as did
 * casts (scalar and vector-to-vector), arithmetic, compound assignment,
 * aggregate initialisers and stores.
 * `(long long)f()` failed even for a bare call. Each consumer x wrapper pair
 * runs for both condition values and is compared against GCC. */
#include <stdio.h>
#include <string.h>
#include <xmmintrin.h>

typedef int T __attribute__((vector_size(8)));
typedef short H __attribute__((vector_size(4)));
typedef float F2 __attribute__((vector_size(8)));
typedef unsigned short U4 __attribute__((vector_size(8)));
struct W { T v; int k; };

static unsigned long long hash = 1469598103934665603ULL;
static void mix(unsigned long long x) { hash = (hash ^ x) * 1099511628211ULL; }
static unsigned long long bits(T v) { unsigned long long x; memcpy(&x, &v, 8); return x; }
static unsigned long long mbits(__m64 v) { unsigned long long x; memcpy(&x, &v, 8); return x; }
#define OUT(x) mix((unsigned long long)(x))
#define OUTV(v) mix(bits(v))

__attribute__((noinline)) static T ret(T a, T b) { return a + b + (T){1, 2}; }
__attribute__((noinline)) static H hret(H a, H b) { return a - b; }
__attribute__((noinline)) static F2 fret(F2 a) { return a * 2.0f; }
__attribute__((noinline)) static T fn_cond(int c, T v0, T v1, T v2) { return c ? v0 : ret(v1, v2); }
__attribute__((noinline)) static T fn_comma(int c, T v1, T v2) { return (c, ret(v1, v2)); }
__attribute__((noinline)) static T fn_shuffle(T v, T m) { return __builtin_shuffle(v, m); }
__attribute__((noinline)) static H fn_shuffle4(H v, H m) { return __builtin_shuffle(v, m); }

int main(void)
{
    for (volatile int c = 0; c < 2; ++c) {
        T v0 = {5, -6}, v1 = {0x10001, 2}, v2 = {3, 0x1fffffff};
        { long long x = (long long)(ret(v1, v2)); OUT(x); } /* cast/call */
        { long long x = (long long)(c ? v0 : ret(v1, v2)); OUT(x); } /* cast/cond */
        { long long x = (long long)((c, ret(v1, v2))); OUT(x); } /* cast/comma */
        { long long x = (long long)(({ T q_ = ret(v2, v1); ret(q_, v2); })); OUT(x); } /* cast/stmt */
        { long long x = (long long)(c ? ret(v0, v1) : ret(v1, v2)); OUT(x); } /* cast/cond2 */
        { long long x = (long long)(c ? (c > 1 ? v1 : ret(v2, v0)) : ret(v1, v2)); OUT(x); } /* cast/nested */
        { T a = (ret(v1, v2)) + v1; OUTV(a); } /* binop/call */
        { T a = (c ? v0 : ret(v1, v2)) + v1; OUTV(a); } /* binop/cond */
        { T a = ((c, ret(v1, v2))) + v1; OUTV(a); } /* binop/comma */
        { T a = (({ T q_ = ret(v2, v1); ret(q_, v2); })) + v1; OUTV(a); } /* binop/stmt */
        { T a = (c ? ret(v0, v1) : ret(v1, v2)) + v1; OUTV(a); } /* binop/cond2 */
        { T a = (c ? (c > 1 ? v1 : ret(v2, v0)) : ret(v1, v2)) + v1; OUTV(a); } /* binop/nested */
        { T a = v1; a += (ret(v1, v2)); OUTV(a); } /* cassign/call */
        { T a = v1; a += (c ? v0 : ret(v1, v2)); OUTV(a); } /* cassign/cond */
        { T a = v1; a += ((c, ret(v1, v2))); OUTV(a); } /* cassign/comma */
        { T a = v1; a += (({ T q_ = ret(v2, v1); ret(q_, v2); })); OUTV(a); } /* cassign/stmt */
        { T a = v1; a += (c ? ret(v0, v1) : ret(v1, v2)); OUTV(a); } /* cassign/cond2 */
        { T a = v1; a += (c ? (c > 1 ? v1 : ret(v2, v0)) : ret(v1, v2)); OUTV(a); } /* cassign/nested */
        { T arr[2] = { ret(v1, v2), v1 }; OUTV(arr[0]); } /* arrayinit/call */
        { T arr[2] = { c ? v0 : ret(v1, v2), v1 }; OUTV(arr[0]); } /* arrayinit/cond */
        { T arr[2] = { (c, ret(v1, v2)), v1 }; OUTV(arr[0]); } /* arrayinit/comma */
        { T arr[2] = { ({ T q_ = ret(v2, v1); ret(q_, v2); }), v1 }; OUTV(arr[0]); } /* arrayinit/stmt */
        { T arr[2] = { c ? ret(v0, v1) : ret(v1, v2), v1 }; OUTV(arr[0]); } /* arrayinit/cond2 */
        { T arr[2] = { c ? (c > 1 ? v1 : ret(v2, v0)) : ret(v1, v2), v1 }; OUTV(arr[0]); } /* arrayinit/nested */
        { T a = -(ret(v1, v2)); OUTV(a); } /* unary/call */
        { T a = -(c ? v0 : ret(v1, v2)); OUTV(a); } /* unary/cond */
        { T a = -((c, ret(v1, v2))); OUTV(a); } /* unary/comma */
        { T a = -(({ T q_ = ret(v2, v1); ret(q_, v2); })); OUTV(a); } /* unary/stmt */
        { T a = -(c ? ret(v0, v1) : ret(v1, v2)); OUTV(a); } /* unary/cond2 */
        { T a = -(c ? (c > 1 ? v1 : ret(v2, v0)) : ret(v1, v2)); OUTV(a); } /* unary/nested */
        { T a = (ret(v1, v2)) == v0; OUTV(a); } /* cmp/call */
        { T a = (c ? v0 : ret(v1, v2)) == v0; OUTV(a); } /* cmp/cond */
        { T a = ((c, ret(v1, v2))) == v0; OUTV(a); } /* cmp/comma */
        { T a = (({ T q_ = ret(v2, v1); ret(q_, v2); })) == v0; OUTV(a); } /* cmp/stmt */
        { T a = (c ? ret(v0, v1) : ret(v1, v2)) == v0; OUTV(a); } /* cmp/cond2 */
        { T a = (c ? (c > 1 ? v1 : ret(v2, v0)) : ret(v1, v2)) == v0; OUTV(a); } /* cmp/nested */
        { struct W s; s.k = 9; s.v = (ret(v1, v2)); OUTV(s.v); OUT(s.k); } /* member/call */
        { struct W s; s.k = 9; s.v = (c ? v0 : ret(v1, v2)); OUTV(s.v); OUT(s.k); } /* member/cond */
        { struct W s; s.k = 9; s.v = ((c, ret(v1, v2))); OUTV(s.v); OUT(s.k); } /* member/comma */
        { struct W s; s.k = 9; s.v = (({ T q_ = ret(v2, v1); ret(q_, v2); })); OUTV(s.v); OUT(s.k); } /* member/stmt */
        { struct W s; s.k = 9; s.v = (c ? ret(v0, v1) : ret(v1, v2)); OUTV(s.v); OUT(s.k); } /* member/cond2 */
        { struct W s; s.k = 9; s.v = (c ? (c > 1 ? v1 : ret(v2, v0)) : ret(v1, v2)); OUTV(s.v); OUT(s.k); } /* member/nested */
        { T t = v0, *p = &t; *p = (ret(v1, v2)); OUTV(t); } /* deref/call */
        { T t = v0, *p = &t; *p = (c ? v0 : ret(v1, v2)); OUTV(t); } /* deref/cond */
        { T t = v0, *p = &t; *p = ((c, ret(v1, v2))); OUTV(t); } /* deref/comma */
        { T t = v0, *p = &t; *p = (({ T q_ = ret(v2, v1); ret(q_, v2); })); OUTV(t); } /* deref/stmt */
        { T t = v0, *p = &t; *p = (c ? ret(v0, v1) : ret(v1, v2)); OUTV(t); } /* deref/cond2 */
        { T t = v0, *p = &t; *p = (c ? (c > 1 ? v1 : ret(v2, v0)) : ret(v1, v2)); OUTV(t); } /* deref/nested */
        { T a = (ret(v1, v2)); OUTV(a); } /* init/call */
        { T a = (c ? v0 : ret(v1, v2)); OUTV(a); } /* init/cond */
        { T a = ((c, ret(v1, v2))); OUTV(a); } /* init/comma */
        { T a = (({ T q_ = ret(v2, v1); ret(q_, v2); })); OUTV(a); } /* init/stmt */
        { T a = (c ? ret(v0, v1) : ret(v1, v2)); OUTV(a); } /* init/cond2 */
        { T a = (c ? (c > 1 ? v1 : ret(v2, v0)) : ret(v1, v2)); OUTV(a); } /* init/nested */
        { T a; a = (ret(v1, v2)); OUTV(a); } /* assign/call */
        { T a; a = (c ? v0 : ret(v1, v2)); OUTV(a); } /* assign/cond */
        { T a; a = ((c, ret(v1, v2))); OUTV(a); } /* assign/comma */
        { T a; a = (({ T q_ = ret(v2, v1); ret(q_, v2); })); OUTV(a); } /* assign/stmt */
        { T a; a = (c ? ret(v0, v1) : ret(v1, v2)); OUTV(a); } /* assign/cond2 */
        { T a; a = (c ? (c > 1 ? v1 : ret(v2, v0)) : ret(v1, v2)); OUTV(a); } /* assign/nested */
        { OUT((ret(v1, v2))[1]); } /* subscript/call */
        { OUT((c ? v0 : ret(v1, v2))[1]); } /* subscript/cond */
        { OUT(((c, ret(v1, v2)))[1]); } /* subscript/comma */
        { OUT((({ T q_ = ret(v2, v1); ret(q_, v2); }))[1]); } /* subscript/stmt */
        { OUT((c ? ret(v0, v1) : ret(v1, v2))[1]); } /* subscript/cond2 */
        { OUT((c ? (c > 1 ? v1 : ret(v2, v0)) : ret(v1, v2))[1]); } /* subscript/nested */
        { OUTV(ret(v1, v2)); } /* arg/call */
        { OUTV(c ? v0 : ret(v1, v2)); } /* arg/cond */
        { OUTV((c, ret(v1, v2))); } /* arg/comma */
        { OUTV(({ T q_ = ret(v2, v1); ret(q_, v2); })); } /* arg/stmt */
        { OUTV(c ? ret(v0, v1) : ret(v1, v2)); } /* arg/cond2 */
        { OUTV(c ? (c > 1 ? v1 : ret(v2, v0)) : ret(v1, v2)); } /* arg/nested */
        /* Other small vector shapes: 4-byte integer, 8-byte float, and the
         * GCC shuffle builtin, whose 8-byte result is also a packed carrier. */
        H h0 = {1, -2}, h1 = {300, 4};
        H h = c ? h0 : hret(h0, h1);
        unsigned hb; memcpy(&hb, &h, 4); OUT(hb);
        F2 f0 = {1.5f, -2.25f};
        F2 f = c ? f0 : fret(f0);
        OUT((long long)f);
        T m = {1, 0};
        T s = c ? v1 : __builtin_shuffle(v1, v2, m);
        OUTV(s);
        OUTV(fn_cond(c, v0, v1, v2));
        OUTV(fn_comma(c, v1, v2));
        OUTV(fn_shuffle(v2, m));
        H hs = fn_shuffle4(h1, (H){1, 0});
        memcpy(&hb, &hs, 4); OUT(hb);
        /* A vector-to-vector cast reinterprets the bits of its operand; the
         * same-size path forwarded a packed call result as the cast's value,
         * which every consumer then dereferenced as an address. */
        U4 u = (U4)ret(v1, v2);
        OUT(u[3]);
        OUT(((U4)ret(v0, v2))[1]);
        OUTV((T)(c ? fret(f0) : f0));
        OUTV((T)(c, fret(f0)));
        /* __m64 is an 8-byte vector: the intrinsics take the same paths. */
        __m64 a = _mm_set_pi16(1, 2, 3, 4), b = _mm_set_pi16(-5, 6, -7, 8);
        __m64 r = c ? a : _mm_add_pi16(a, b);
        OUT(mbits(r));
        OUT(mbits(c ? _mm_sub_pi16(a, b) : b));
        OUT(mbits((c, _mm_xor_si64(a, b))));
        OUT(mbits((__m64)_mm_add_pi16(a, b)));
        OUT(_mm_extract_pi16(_mm_add_pi16(a, b), 1));
    }
    _mm_empty();
    printf("small vector packed wrappers %016llx\n", hash);
    return 0;
}
