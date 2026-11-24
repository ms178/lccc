/* `_Generic` tells a pointer to an atomic object from a pointer to a plain
 * or a volatile one (C11 6.5.1.1p2: the association's type must be
 * compatible with the operand's, and `_Atomic int` is compatible with
 * neither `int` nor `volatile int`).
 *
 * lccc's types had no record of `_Atomic`, and its lowering compared no
 * qualifier but `const`: `_Generic(&counter, volatile int *: ..., int *:
 * ..., default: ...)` took the first association of the right shape,
 * `volatile int *`, for an atomic `counter` (GCC and Clang: `default`) and
 * for a plain `int` too (GCC and Clang: `int *`). <tgmath.h>-style
 * dispatch macros that separate the three picked the wrong function.
 *
 * Every spelling an object can get its qualifiers from is here: the
 * qualifier, the `_Atomic(T)` specifier, a typedef, a star's own qualifier,
 * a member, an element, an array that decays, and `typeof`. */
#include <stdio.h>

_Atomic int a;
int n;
volatile int v;
const _Atomic int ca = 1;
const int cn = 1;
_Atomic(int *) ap;
int *_Atomic ap2;
typedef _Atomic int aint;
aint ta;
typedef volatile int vint;
vint tv;
struct S {
    _Atomic int m;
    int *_Atomic pm;
    volatile int vm;
    int pl;
    const int cm;
    aint tm;
    _Atomic int arr[2];
    _Atomic(char *) text;
} s;
struct S *sp = &s;
_Atomic int aarr[3];
int narr[3];
volatile int varr[3];

#define G(e)                                                                               \
    _Generic(e, volatile int *: 1, int *: 2, _Atomic int *: 4, const _Atomic int *: 6,     \
             const int *: 7, int **: 8, int *_Atomic *: 9, _Atomic(char *) *: 10, default: 3)
/* No atomic association: an atomic operand has only `default` to take. */
#define H(e) _Generic(e, volatile int *: 1, int *: 2, const int *: 7, default: 3)
/* The same associations in another order: the first compatible one is the
 * only compatible one, so the order must not matter. */
#define R(e) _Generic(e, default: 3, _Atomic int *: 4, int *: 2, volatile int *: 1)

int main(void) {
    printf("objects: %d %d %d %d %d %d %d %d %d\n", G(&a), G(&n), G(&v), G(&ca), G(&cn), G(&ap),
           G(&ap2), G(&ta), G(&tv));
    printf("members: %d %d %d %d %d %d %d %d %d %d\n", G(&s.m), G(&s.pm), G(&s.vm), G(&s.pl),
           G(&s.cm), G(&s.tm), G(&sp->m), G(&sp->vm), G(&sp->pl), G(&sp->text));
    printf("arrays: %d %d %d %d %d %d %d\n", G(&s.arr[1]), G(s.arr), G(aarr), G(&aarr[0]),
           G(narr), G(varr), G(sp->arr));
    printf("no atomic association: %d %d %d %d %d %d %d %d\n", H(&a), H(&n), H(&v), H(&s.m),
           H(&s.vm), H(&s.pl), H(&sp->pl), H(aarr));
    printf("reordered: %d %d %d %d %d %d\n", R(&a), R(&n), R(&v), R(&s.m), R(&sp->pl), R(aarr));
    /* Lvalue conversion drops every qualifier, `_Atomic` included. */
    printf("values: %d %d %d %d\n", _Generic(a, int: 1, default: 2),
           _Generic(ap, int *: 1, default: 2), _Generic(s.m, int: 1, default: 2),
           _Generic(+a, int: 1, default: 2));
    printf("typeof: %d %d %d %d\n",
           _Generic((__typeof__(a) *)0, int *: 2, _Atomic int *: 4, default: 3),
           _Generic((__typeof__(v) *)0, int *: 2, volatile int *: 1, default: 3),
           _Generic((__typeof__(n) *)0, volatile int *: 1, int *: 2, default: 3),
           _Generic((__typeof__(s.m) *)0, int *: 2, _Atomic int *: 4, default: 3));
    return 0;
}
