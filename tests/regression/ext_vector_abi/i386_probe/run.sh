#!/bin/bash
# usage: run.sh "ELEM:LSIZE" "lanes..."   (callee compiled by $CC, default clang-23)
CC=${CC:-clang-23}
cd "$(dirname "$0")"
clang-23 -m32 -O1 -msse2 -c harness.c -o h.o || exit 1
for el in $1; do E=${el%%:*}; E=${E//__/ }; S=${el##*:}; for n in $2; do
  if ! $CC -m32 -O2 -w -c -DELEM="$E" -DLANES=$n -DLSIZE=$S callee.c -o c.o 2>/dev/null; then echo "$E x$n: CE"; continue; fi
  clang-23 -m32 h.o c.o -o p 2>/dev/null || { echo "$E x$n: LINK"; continue; }
  printf '%-22s ' "$E x$n"; ./p | tr '\n' ' '; echo
done; done
