#!/usr/bin/env bash
# IS-03: AVX2 64-byte assignment uses two YMM load/store pairs plus vzeroupper;
# baseline targets remain free of AVX instructions.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-copy64.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
cat >"$tmp/t.c" <<'C'
typedef struct { unsigned long x[8]; } S;
__attribute__((noinline)) void copy64(S *d,const S *s){*d=*s;}
int main(void){S a={{1,2,3,4,5,6,7,8}},b={{0}};copy64(&b,&a);
for(int i=0;i<8;i++)if(a.x[i]!=b.x[i])return 1;return 0;}
C
"$CCC" -O2 -march=x86-64-v3 -S "$tmp/t.c" -o "$tmp/v3.s"
"$CCC" -O2 -march=x86-64-v3 "$tmp/t.c" -o "$tmp/t"
"$tmp/t"
body=$(sed -n '/^copy64:/,/^\.size copy64/p' "$tmp/v3.s")
[[ $(grep -c 'vmovdqu.*%ymm' <<<"$body") -eq 4 ]]
grep -q 'vzeroupper' <<<"$body"
! grep -Eq '(subq|addq).*%rsp|movq[[:space:]]+%rsi,[[:space:]]*%rax' <<<"$body"
[[ $(grep -Ec '^[[:space:]]+(v?mov|vzeroupper|ret)' <<<"$body") -eq 6 ]]
"$CCC" -O2 -S "$tmp/t.c" -o "$tmp/base.s"
! grep -q '%ymm' "$tmp/base.s"
