/* GCC `vector_size` types wider than 16 bytes: TYPE_ALIGN is the full vector
 * size on x86 (struct layout, object alignment, `__alignof__`, parameter
 * passing), while C11 `_Alignof(type)` is GCC's `min_align_of_type` -- capped
 * at BIGGEST_ALIGNMENT (16 without AVX) unless the type is user-aligned.
 * Every number below is checked against the GCC reference build by the
 * regression runner; the runtime half exercises the >16-byte MEMORY-class
 * argument slots (variadic and named) on both stack residues mod 32/64,
 * which is where ABI-EXTVEC-VA-1 read an unwritten slot.
 *
 * Measured on GCC 16.2 (-m64 and -m32 agree on every value here). */
#include <stdarg.h>
#include <stdio.h>

typedef unsigned v32 __attribute__((vector_size(32)));
typedef unsigned v64 __attribute__((vector_size(64)));
typedef unsigned char c32 __attribute__((vector_size(32)));

struct S32 { int z; v32 v; };
struct S64 { int z; v64 v; };
struct A { _Alignas(32) int x; };          /* user-aligned: no cap          */
struct N { struct A a; };                   /* user-ness propagates           */
struct SN { struct S32 s; };                /* vector-ness propagates, capped */
struct __attribute__((aligned(8))) SA8 { v32 v; };   /* attribute => user   */
struct SF { v32 v __attribute__((aligned(8))); };     /* below type: not user */
union U { int i; v64 v; };

v32 gv;
struct S32 gs;
v64 garr[2];

static void layout(void)
{
    printf("v32 %zu %zu %zu\n", sizeof(v32), _Alignof(v32), __alignof__(v32));
    printf("v64 %zu %zu %zu\n", sizeof(v64), _Alignof(v64), __alignof__(v64));
    printf("c32 %zu %zu %zu\n", sizeof(c32), _Alignof(c32), __alignof__(c32));
    printf("S32 %zu %zu %zu off %zu\n", sizeof(struct S32), _Alignof(struct S32),
           __alignof__(struct S32), __builtin_offsetof(struct S32, v));
    printf("S64 %zu %zu %zu off %zu\n", sizeof(struct S64), _Alignof(struct S64),
           __alignof__(struct S64), __builtin_offsetof(struct S64, v));
    printf("A %zu %zu\n", sizeof(struct A), _Alignof(struct A));
    printf("N %zu %zu\n", sizeof(struct N), _Alignof(struct N));
    printf("SN %zu %zu %zu\n", sizeof(struct SN), _Alignof(struct SN), __alignof__(struct SN));
    printf("SA8 %zu %zu\n", sizeof(struct SA8), _Alignof(struct SA8));
    printf("SF %zu %zu\n", sizeof(struct SF), _Alignof(struct SF));
    printf("U %zu %zu %zu\n", sizeof(union U), _Alignof(union U), __alignof__(union U));
    printf("arr %zu %zu\n", _Alignof(v32[2]), _Alignof(garr));
    printf("expr %zu %zu\n", _Alignof(gv), _Alignof(gs));
    printf("objects %d %d %d\n", (int)((unsigned long)&gv % 32), (int)((unsigned long)&gs % 32),
           (int)((unsigned long)garr % 64));
}

/* --- runtime: MEMORY-class vector arguments at their full alignment --- */

__attribute__((noinline)) static unsigned long long va32(int n, ...)
{
    va_list ap;
    va_start(ap, n);
    v32 v = va_arg(ap, v32);
    int k = va_arg(ap, int);
    va_end(ap);
    return v[0] + v[7] * 1000ull + k * 1000000ull;
}

__attribute__((noinline)) static unsigned long long va64(int n, ...)
{
    va_list ap;
    va_start(ap, n);
    int j = va_arg(ap, int);
    v64 v = va_arg(ap, v64);
    int k = va_arg(ap, int);
    va_end(ap);
    return v[0] + v[15] * 1000ull + k * 1000000ull + j;
}

/* Six ints fill the GP registers; `extra` is the first stack slot, so the
 * vector's slot needs 24 (resp. 56) bytes of padding before it. */
__attribute__((noinline)) static unsigned long long
named32(int a, int b, int c, int d, int e, int f, int extra, v32 v, int k)
{
    return v[0] + v[7] * 1000ull + k * 1000000ull + extra * 1000000000ull + a + b + c + d + e + f;
}

__attribute__((noinline)) static unsigned long long
named64(int a, int b, int c, int d, int e, int f, int extra, v64 v, int k)
{
    return v[0] + v[15] * 1000ull + k * 1000000ull + extra * 1000000000ull + a + b + c + d + e + f;
}

__attribute__((noinline)) static unsigned long long first32(v32 v, int k)
{
    return v[0] + v[7] * 1000ull + k * 1000000ull;
}

__attribute__((noinline)) static int calls(int depth)
{
    v32 a = {1, 2, 3, 4, 5, 6, 7, 9};
    v64 b = {0};
    volatile char pad[8];
    int bad = 0;
    b[0] = 3;
    b[15] = 11;
    pad[0] = (char)depth;
    if (va32(1, a, 17) != 1 + 9 * 1000ull + 17 * 1000000ull) bad |= 1;
    if (va64(3, 5, b, 19) != 3 + 11 * 1000ull + 19 * 1000000ull + 5) bad |= 2;
    if (named32(1, 2, 3, 4, 5, 6, 7, a, 9) != 1 + 9 * 1000ull + 9 * 1000000ull + 7 * 1000000000ull + 21)
        bad |= 4;
    if (named64(1, 2, 3, 4, 5, 6, 7, b, 9) != 3 + 11 * 1000ull + 9 * 1000000ull + 7 * 1000000000ull + 21)
        bad |= 8;
    if (first32(a, 5) != 1 + 9 * 1000ull + 5 * 1000000ull) bad |= 16;
    return bad + pad[0] - depth;
}

/* Shift the frame by one 16-byte slot so the call sites run on both
 * residues of %rsp mod 32 (and all four mod 64 across the two depths). */
__attribute__((noinline)) static int shifted(int depth)
{
    volatile char pad[16];
    pad[0] = 0;
    return calls(depth) + pad[0];
}

int main(void)
{
    layout();
    printf("calls %d %d\n", calls(0), shifted(1));
    return 0;
}
