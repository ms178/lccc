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
