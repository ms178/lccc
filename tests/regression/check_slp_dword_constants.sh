#!/usr/bin/env bash
# Literal-lane operands must not become pointers in the SSE2 multiply fallback.
set -euo pipefail
cd "$(dirname "$0")/../.."
cc=${CCC:-target/fastbuild/lccc}
gcc=${GCC:-gcc-16}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
src=tests/regression/slp_dword_constants.c
"$gcc" -O0 -Wall -Wextra -Werror "$src" -o "$tmp/reference"
expected=$("$tmp/reference")
for opt in -O2 -O3; do
 for isa in sse2 sse41 avx2; do
  case $isa in
   sse2) flags=(-msse2 -mno-sse4.1 -mno-avx);;
   sse41) flags=(-msse4.1 -mno-avx);;
   avx2) flags=(-mavx2);;
  esac
  "$cc" "$opt" "${flags[@]}" -Wall -Wextra -Werror "$src" -o "$tmp/probe"
  "$cc" "$opt" "${flags[@]}" -S "$src" -o "$tmp/probe.s"
  case $isa in sse2) mul=pmuludq;; *) mul=pmulld;; esac
  awk '/^lcg:/{f=1} f{print} f && /^\.size/{exit}' "$tmp/probe.s" > "$tmp/lcg.s"
  grep -Eq "\bv?$mul\b" "$tmp/lcg.s" || {
   echo "FAIL $opt $isa: literal multiply was not vectorized"; exit 1;
  }
  actual=$("$tmp/probe")
  test "$actual" = "$expected" || { echo "FAIL $opt $isa: $actual != $expected"; exit 1; }
  echo "PASS slp constants $opt $isa: $actual"
 done
done
