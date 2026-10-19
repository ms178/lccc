#!/usr/bin/env bash
set -euo pipefail
CCC=${CCC:-./target/release/lccc}; t=$(mktemp -d); trap 'rm -rf "$t"' EXIT
cat >"$t/bad.c" <<'C'
struct A{int x;}; struct B{int x;}; int one(int x){return x;}
void v(void){return 1;} int n(void){return;}
int main(void){struct A a;struct B b;a=b;int *p;double*q;p=q;int(*fp)(int)=one;return one()+one(1,2)+fp();}
C
! "$CCC" -fsyntax-only "$t/bad.c" 2>"$t/e"
for x in "'return' with a value" "return with no value" "incompatible types" "incompatible pointer types" "too few arguments to function 'one'" "too many arguments to function 'one'" "too few arguments to function 'fp'"; do grep -q "$x" "$t/e"; done
cat >"$t/good.c" <<'C'
struct A{int x;}; static int add(int a,int b){return a+b;} static int sum(int n,...){return n;}
int main(void){struct A a={1},b=a;void*vp=&a;struct A*ap=vp;int(*fp)(int,int)=add;return fp(a.x,b.x)==2&&sum(2,1,2)==2&&ap==&a?0:1;}
C
"$CCC" -O2 "$t/good.c" -o "$t/g"; "$t/g"

# Lvalue constraints (C11 6.5.16p2, 6.5.2.4p1, 6.5.3.1p1, 6.5.3.2p1), Clang
# 23.1.3's spellings. Each line used to compile to nothing (the store went
# nowhere); GCC 16.2 and Clang reject every one.
cat >"$t/lv_bad.c" <<'C'
struct S{int m;}; struct S sf(void){struct S s={0}; return s;} int fi(void){return 1;}
int x,y,c; struct S s1,s2;
void a1(void){ fi() = 1; }
void a2(void){ x + 1 = 2; }
void a3(void){ (x, y) = 3; }
void a4(void){ (c ? x : y) = 4; }
void a5(void){ sf().m = 5; }
void a6(void){ sf().m += 6; }
void a7(void){ sf().m++; }
void a8(void){ --(c ? s1 : s2).m; }
void a9(void){ sf() = s1; }
void a10(void){ __builtin_choose_expr(1, fi(), y) = 10; }
void a11(void){ __real__ (x + 1) = 11; }
void c1(void){ ((int)x) = 1; }
void c2(void){ ((int)x)++; }
int *d1(void){ return &sf().m; }
int *d2(void){ return &(x + 1); }
int *d3(void){ return &(int)x; }
C
! "$CCC" -fsyntax-only "$t/lv_bad.c" 2>"$t/lv_e"
[ "$(grep -c "error: expression is not assignable" "$t/lv_e")" = 11 ]
[ "$(grep -c "error: assignment to cast is illegal, lvalue casts are not supported" "$t/lv_e")" = 2 ]
[ "$(grep -c "error: cannot take the address of an rvalue of type 'int'" "$t/lv_e")" = 3 ]
# The lvalue forms stay assignable, and the stores land: `__builtin_choose_expr`
# and `_Generic` keep the chosen operand's category (GCC), a GNU statement
# expression its tail's, `__real__`/`__imag__` of an lvalue are lvalues.
cat >"$t/lv_good.c" <<'C'
struct S{int m; int a[2];}; struct S sf(void){struct S s={0,{0,0}}; return s;}
int x, y, *p = &y; struct S s; _Complex double z;
int main(void){
  __builtin_choose_expr(1, x, y) = 5; __builtin_choose_expr(0, x, y) += 7;
  __builtin_choose_expr(1, s, s).m = 4; _Generic(s, struct S: s).m += 3;
  *&__builtin_choose_expr(1, x, y) += 100; _Generic(x, int: x) *= 2;
  ({ s; }).a[1] = 9; *p += 1; __real__ z = 1.5; __imag__ z = 2.5;
  (&s)->a[0] = sf().a[0] + 2; ((struct S){1, {2, 3}}).m = 4;
  return x == 210 && y == 8 && s.m == 7 && s.a[0] == 2 && s.a[1] == 9
         && __real__ z == 1.5 && __imag__ z == 2.5 ? 0 : 1;
}
C
for o in -O0 -O2; do "$CCC" $o "$t/lv_good.c" -o "$t/lv_g"; "$t/lv_g"; done

# `__builtin_choose_expr` takes an integer constant expression (C11 6.6p6,
# `ice.rs`): an object -- a `const` one, a local hiding an enumerator --
# comma, a string subscript, an address, an unevaluated non-constant arm,
# a float outside an integer cast, an undefined value, a VLA `sizeof` or a
# non-folding builtin is not one. Every form measured with gcc-16 -std=gnu17
# (15 errors); lccc used to fold several of them and pick an arm.
cat >"$t/ch_bad.c" <<'C'
struct P { int a, b; }; enum { E1 = 1, E_shadow = 2 }; int gk; const char *s;
int b1(int x){ const int k = 1; int E_shadow = 0; (void)k; (void)E_shadow; return __builtin_choose_expr(k, 1, 2); }
int b2(int x){ const int k = 1; int E_shadow = 0; (void)k; (void)E_shadow; return __builtin_choose_expr(x - x, 1, 2); }
int b3(int x){ const int k = 1; int E_shadow = 0; (void)k; (void)E_shadow; return __builtin_choose_expr((1, 1), 1, 2); }
int b4(int x){ const int k = 1; int E_shadow = 0; (void)k; (void)E_shadow; return __builtin_choose_expr("ab"[0], 1, 2); }
int b5(int x){ const int k = 1; int E_shadow = 0; (void)k; (void)E_shadow; return __builtin_choose_expr((void *)0 == 0, 1, 2); }
int b6(int x){ const int k = 1; int E_shadow = 0; (void)k; (void)E_shadow; return __builtin_choose_expr(&gk != 0, 1, 2); }
int b7(int x){ const int k = 1; int E_shadow = 0; (void)k; (void)E_shadow; return __builtin_choose_expr(1 ? 2 : x, 1, 2); }
int b8(int x){ const int k = 1; int E_shadow = 0; (void)k; (void)E_shadow; return __builtin_choose_expr(0 && x, 1, 2); }
int b9(int x){ const int k = 1; int E_shadow = 0; (void)k; (void)E_shadow; return __builtin_choose_expr(1.5, 1, 2); }
int b10(int x){ const int k = 1; int E_shadow = 0; (void)k; (void)E_shadow; return __builtin_choose_expr((int)1e100, 1, 2); }
int b11(int x){ const int k = 1; int E_shadow = 0; (void)k; (void)E_shadow; return __builtin_choose_expr(1 / 0, 1, 2); }
int b12(int x){ const int k = 1; int E_shadow = 0; (void)k; (void)E_shadow; return __builtin_choose_expr(1 << 40, 1, 2); }
int b13(int x){ const int k = 1; int E_shadow = 0; (void)k; (void)E_shadow; return __builtin_choose_expr(sizeof(int[x]), 1, 2); }
int b14(int x){ const int k = 1; int E_shadow = 0; (void)k; (void)E_shadow; return __builtin_choose_expr(__builtin_strlen(s), 1, 2); }
int b15(int x){ const int k = 1; int E_shadow = 0; (void)k; (void)E_shadow; return __builtin_choose_expr(E_shadow, 1, 2); }
C
! "$CCC" -std=gnu17 -fsyntax-only "$t/ch_bad.c" 2>"$t/ch_e"
[ "$(grep -c "first argument to '__builtin_choose_expr' not a constant" "$t/ch_e")" = 15 ]
# The accepted forms select GCC's arm. `__builtin_constant_p` of a parameter
# is 0 there (the kernel idiom; lccc took the constant arm once inlined),
# `__builtin_offsetof` is a constant while the spelled-out `&((T *)0)->m`
# cast is not, and C90 still admits an overflowing signed shift.
cat >"$t/ch_good.c" <<'C'
#include <stddef.h>
struct P { int a, b; }; enum { E1 = 1 };
__attribute__((noinline)) int kern(int x) { return __builtin_choose_expr(__builtin_constant_p(x), 100, x); }
int main(void) {
  int x = 3, r = 0;
  r += __builtin_choose_expr(E1, 1, 2) + __builtin_choose_expr(sizeof(long) == 8, 10, 20);
  r += __builtin_choose_expr(__builtin_types_compatible_p(int, long), 1000, 100);
  r += __builtin_choose_expr(_Generic(x, int: 1, default: 0), 1, 2) + __builtin_choose_expr((int)1.5, 1, 2);
  r += __builtin_choose_expr(__builtin_popcount(7) == 3 && __builtin_strlen("ab") == 2, 1, 2);
  r += __builtin_choose_expr(1 ? 2 : 3, 1, 2) + __builtin_choose_expr(0 && 1 / 0, 2, 1);
  r += __builtin_choose_expr(offsetof(struct P, b) == 4, 1, 2) + __builtin_choose_expr(__builtin_LINE() > 0, 1, 2);
  return r == (sizeof(long) == 8 ? 10 : 20) + 108 && kern(5) == 5 ? 0 : 1;
}
C
for o in -O0 -O2; do "$CCC" $o "$t/ch_good.c" -o "$t/ch_g"; "$t/ch_g"; done
printf 'int f(void) { return __builtin_choose_expr(1 << 31, 1, 2); }\n' >"$t/ch89.c"
"$CCC" -std=gnu89 -fsyntax-only "$t/ch89.c"
! "$CCC" -std=gnu17 -fsyntax-only "$t/ch89.c" 2>/dev/null
printf 'struct P { int a, b; };\nint f(void) { return __builtin_choose_expr((long)&((struct P *)0)->b, 1, 2); }\n' >"$t/ch_off.c"
! "$CCC" -fsyntax-only "$t/ch_off.c" 2>/dev/null
# Enumerator scope in the parser's `_Static_assert` evaluator: an object
# hides an enumerator, a block-scope enumerator ends with its block.
printf 'enum { A = 1 };\nint h(void) { int A = 5; _Static_assert(A, "x"); return A; }\n' >"$t/en_shadow.c"
printf 'void f(void) { enum { X = 1 }; }\n_Static_assert(X, "leak");\n' >"$t/en_leak.c"
for f in en_shadow en_leak; do
  ! "$CCC" -fsyntax-only "$t/$f.c" 2>"$t/$f.e"
  grep -q "expression in static assertion is not an integer constant expression" "$t/$f.e"
done
