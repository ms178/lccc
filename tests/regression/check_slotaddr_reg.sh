#!/usr/bin/env bash
# RA-24: register-resident pointer homes are explicit SlotAddr::Reg values.
set -euo pipefail
CCC=${CCC:-./target/release/lccc}
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
t=$(mktemp -d); trap 'rm -rf "$t"' EXIT
cat >"$t/t.c" <<'C'
typedef struct { unsigned long x[8]; } S;
__attribute__((noinline)) int rd(const volatile int *p){return p[3];}
__attribute__((noinline)) void wr(volatile int *p,int v){p[5]=v;}
__attribute__((noinline)) unsigned long long rd64(const volatile unsigned long long *p){return p[2];}
__attribute__((noinline)) void wr64(volatile unsigned long long *p,unsigned long long v){p[1]=v;}
__attribute__((noinline)) double rdf(const volatile double *p){return p[2];}
__attribute__((noinline)) void wrf(volatile double *p,double v){p[1]=v;}
__attribute__((noinline)) long double rdld(const volatile long double *p){return p[1];}
__attribute__((noinline)) void wrld(volatile long double *p,long double v){p[1]=v;}
__attribute__((noinline)) void cp(S*d,const S*s){*d=*s;}
int main(void){
 int a[8]={0}; unsigned long long q[4]={0}; double d[4]={0};
 S x={{1,2,3,4,5,6,7,8}},y={{0}};
 wr(a,17);wr64(q,0x123456789abcdef0ULL);wrf(d,3.25);cp(&y,&x);
 if(rd(a)!=0||a[5]!=17||rd64(q)!=0||q[1]!=0x123456789abcdef0ULL)return 1;
 if(rdf(d)!=0.0||d[1]!=3.25||y.x[7]!=8)return 2;
 return 0;
}
C
"$CCC" -O2 -march=x86-64-v3 "$t/t.c" -o "$t/run"; "$t/run"
"$CCC" -O2 -march=x86-64-v3 -S "$t/t.c" -o "$t/x64.s"
grep -Eq '12\(%r(di|si|dx|cx|8|9|1[0-5])\)' "$t/x64.s"
! grep -Eq '(mov|fld|fst).*[[:space:]]0\(%rbp\)' "$t/x64.s"
! grep -R -q 'SlotAddr::Indirect(StackSlot(0))' "$root/src/backend"

bindir=$(dirname "$CCC")
for target in arm riscv i686; do
  bin="$bindir/lccc-$target"
  [ -x "$bin" ] || continue
  "$bin" -O2 -S "$t/t.c" -o "$t/$target.s"
  test -s "$t/$target.s"
done
