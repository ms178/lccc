/* `__attribute__((aligned(N)))` on a TYPEDEF replaces the type's alignment --
 * GCC documents that it "can both increase and decrease alignment" -- and
 * that reduced alignment drives struct layout, `_Alignof`/`__alignof__` and
 * `_Alignof(object)`. The pattern is common in ABI headers (`typedef __u64
 * __attribute__((aligned(4))) compat_u64;`, `unaligned_u64`), so a struct
 * holding one must be laid out exactly as GCC does: `struct { char c; D4 d;
 * }` is 12 bytes with `d` at 4, not 16/8.
 *
 * A member-level `aligned(N)` below the type's alignment is different: it is
 * ignored (GCC `do_type_align`), so `struct E` keeps `d` at 16 only because
 * 16 is above both. Reference numbers measured on GCC 16.2, -m64 and -m32. */
#include <stdio.h>

typedef double D4 __attribute__((aligned(4)));
typedef long long L2 __attribute__((aligned(2)));
typedef unsigned v32 __attribute__((vector_size(32)));
typedef v32 V8 __attribute__((aligned(8)));
typedef int I16 __attribute__((aligned(16)));

struct A { char c; D4 d; };
struct B { char c; L2 l; };
struct C { char c; V8 v; };
struct E { char c; D4 d __attribute__((aligned(16))); };
struct F { char c; I16 i; };

D4 gd;
V8 gv;
struct C gc;

__attribute__((noinline)) static v32 addv(struct C *p, v32 x) { return p->v + x; }
__attribute__((noinline)) static void storev(struct C *p, v32 x) { p->v = x; }

int main(void)
{
    printf("D4 %zu %zu %zu\n", sizeof(D4), _Alignof(D4), __alignof__(D4));
    printf("L2 %zu %zu\n", sizeof(L2), _Alignof(L2));
    printf("V8 %zu %zu %zu\n", sizeof(V8), _Alignof(V8), __alignof__(V8));
    printf("I16 %zu %zu\n", sizeof(I16), _Alignof(I16));
    printf("A %zu %zu off %zu\n", sizeof(struct A), _Alignof(struct A),
           __builtin_offsetof(struct A, d));
    printf("B %zu %zu off %zu\n", sizeof(struct B), _Alignof(struct B),
           __builtin_offsetof(struct B, l));
    printf("C %zu %zu off %zu\n", sizeof(struct C), _Alignof(struct C),
           __builtin_offsetof(struct C, v));
    printf("E %zu %zu off %zu\n", sizeof(struct E), _Alignof(struct E),
           __builtin_offsetof(struct E, d));
    printf("F %zu %zu off %zu\n", sizeof(struct F), _Alignof(struct F),
           __builtin_offsetof(struct F, i));
    printf("objects %zu %zu %zu\n", _Alignof(gd), _Alignof(gv), _Alignof(gc));

    /* The 8-aligned vector member must be accessed with unaligned moves. */
    struct C c;
    v32 one = {1, 1, 1, 1, 1, 1, 1, 1};
    unsigned s = 0;
    c.c = 'x';
    storev(&c, one);
    storev(&gc, one + one);
    v32 r = addv(&c, one) + addv(&gc, one);
    for (int i = 0; i < 8; i++) s += r[i];
    printf("sum %u c %c\n", s, c.c);
    return 0;
}
