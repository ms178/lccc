#!/usr/bin/env bash
# IS-12: fuse adjacent single-use `not` + `and` only when BMI1 is enabled.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-andn.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
cat >"$tmp/t.c" <<'C'
__attribute__((noinline)) unsigned long f(unsigned long a, unsigned long b)
{ return a & ~b; }
int main(void) { return f(0xf0f0UL,0x0ff0UL) != 0xf000UL; }
C
"$CCC" -O2 -march=x86-64-v3 -S "$tmp/t.c" -o "$tmp/bmi.s"
"$CCC" -O2 -march=x86-64-v3 "$tmp/t.c" -o "$tmp/t"
"$tmp/t"
body=$(sed -n '/^f:/,/^\.size f/p' "$tmp/bmi.s")
grep -q 'andnq' <<<"$body"
! grep -q 'notq' <<<"$body"
"$CCC" -O2 -S "$tmp/t.c" -o "$tmp/base.s"
! grep -q 'andnq' "$tmp/base.s"  # baseline x86-64 must remain SIGILL-safe
