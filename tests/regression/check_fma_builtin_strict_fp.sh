#!/usr/bin/env bash
# Strict FP must still inline the builtin-backed glibc fma/sqrt definitions.
set -euo pipefail
LCCC=${LCCC:-target/fastbuild/lccc}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cat >"$tmp/test.c" <<'EOF'
#include <stdio.h>
__attribute__((noinline)) double fma_wrap(double a,double b,double c) {
  return __builtin_fma(a,b,c);
}
__attribute__((noinline)) float fmaf_wrap(float a,float b,float c) {
  return __builtin_fmaf(a,b,c);
}
__attribute__((noinline)) double sqrt_wrap(double a) { return __builtin_sqrt(a); }
int main(void) {
  double f=fma_wrap(1.0+0x1p-52,1.0-0x1p-52,-1.0);
  return !(f == -0x1p-104 && fmaf_wrap(2,3,4)==10 && sqrt_wrap(4)==2);
}
EOF
"$LCCC" -O2 -march=x86-64-v3 -frounding-math -ftrapping-math "$tmp/test.c" -o "$tmp/test"
"$tmp/test"
objdump -d "$tmp/test" >"$tmp/dis"
grep -Eq 'vfmadd[0-9]+sd' "$tmp/dis"
grep -Eq 'vfmadd[0-9]+ss' "$tmp/dis"
grep -Eq 'vsqrtsd|sqrtsd' "$tmp/dis"
! sed -n '/<fma_wrap>:/,/^$/p' "$tmp/dis" | grep -E 'call.*fma' >/dev/null
! sed -n '/<sqrt_wrap>:/,/^$/p' "$tmp/dis" | grep -E 'call.*sqrt' >/dev/null
