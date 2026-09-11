#!/usr/bin/env bash
# MS-02: deterministic one-line allocation statistics for benchmark automation.
set -euo pipefail
CCC=${CCC:-./target/release/lccc}
t=$(mktemp -d); trap 'rm -rf "$t"' EXIT
cat >"$t/t.c" <<'C'
__attribute__((noinline)) long pressure(long *p,long n){
 long a=1,b=2,c=3,d=4,e=5,f=6,g=7,h=8;
 for(long i=0;i<n;i++){a+=p[i];b+=a;c+=b;d+=c;e+=d;f+=e;g+=f;h+=g;}
 return a+b+c+d+e+f+g+h;
}
int main(void){long p[2]={1,2};return pressure(p,2)==3033?0:0;}
C
CCC_TRACE_ALLOCSTATS=pressure "$CCC" -O2 -S "$t/t.c" -o "$t/t.s" 2>"$t/log"
line=$(grep '^\[RA-STATS\] fn=pressure ' "$t/log")
[[ $(grep -c '^\[RA-STATS\] fn=pressure ' "$t/log") -eq 1 ]]
for key in eligible scan assigned spilled segments holes callee-homes caller-homes; do
  grep -Eq "${key}=[0-9]+" <<<"$line"
done
# A nonmatching filter must be quiet and must not alter assembly.
CCC_TRACE_ALLOCSTATS=not_pressure "$CCC" -O2 -S "$t/t.c" -o "$t/t2.s" 2>"$t/log2"
! grep -q '^\[RA-STATS\]' "$t/log2"
cmp "$t/t.s" "$t/t2.s"
