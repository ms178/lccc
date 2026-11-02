#!/bin/bash
CC=${CC:-clang-23}
cd "$(dirname "$0")"
clang-23 -m32 -O1 -msse2 -c rharness.c -o rh.o || exit 1
for el in $1; do E=${el%%:*}; E=${E//__/ }; S=${el##*:}; for n in $2; do
  $CC -m32 -O2 -w -c -DELEM="$E" -DLANES=$n -DLSIZE=$S caller.c -o cc.o 2>/dev/null || { echo "$E x$n: CE"; continue; }
  clang-23 -m32 rh.o cc.o -o rp 2>/dev/null || { echo "$E x$n: LINK"; continue; }
  printf '%-22s ' "$E x$n"; ./rp
done; done
