#!/usr/bin/env bash
set -euo pipefail
CCC=${CCC:-./target/release/lccc}; t=$(mktemp -d); trap 'rm -rf "$t"' EXIT
cat >"$t/bad.c" <<'C'
struct A{int x;}; struct B{int x;}; int one(int x){return x;}
void v(void){return 1;} int n(void){return;}
int main(void){struct A a;struct B b;a=b;int *p;double*q;p=q;int(*fp)(int)=one;return one()+one(1,2)+fp();}
C
! "$CCC" -fsyntax-only "$t/bad.c" 2>"$t/e"
for x in "return with a value" "return with no value" "incompatible types" "incompatible pointer types" "too few arguments to function 'one'" "too many arguments to function 'one'" "too few arguments to function 'function pointer'"; do grep -q "$x" "$t/e"; done
cat >"$t/good.c" <<'C'
struct A{int x;}; static int add(int a,int b){return a+b;} static int sum(int n,...){return n;}
int main(void){struct A a={1},b=a;void*vp=&a;struct A*ap=vp;int(*fp)(int,int)=add;return fp(a.x,b.x)==2&&sum(2,1,2)==2&&ap==&a?0:1;}
C
"$CCC" -O2 "$t/good.c" -o "$t/g"; "$t/g"
