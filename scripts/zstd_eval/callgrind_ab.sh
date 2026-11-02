#!/bin/sh
# Callgrind Ir A/B of zstd CLI binaries on one input at levels 1/3/19
# (compression: the cache model of scripts/benchmark_zstd.py). Every
# decompression is checked byte-exact against the input.
# usage: callgrind_ab.sh <input> <name=zstd-binary>...
in=$1; shift; T=${TMPDIR:-/tmp}/zcg.$$; mkdir -p "$T"
total() { callgrind_annotate "$1" 2>/dev/null | awk '/PROGRAM TOTALS/{gsub(",","",$1); print $1}'; }
for l in 1 3 19; do for nb in "$@"; do n=${nb%%=*}; b=${nb#*=}
  "$b" -q -f -$l "$in" -o "$T/ref$l.zst" 2>/dev/null
  valgrind --tool=callgrind --I1=32768,8,64 --D1=32768,8,64 --LL=33554432,16,64 \
      --callgrind-out-file="$T/c$l.$n" "$b" -q -f -$l "$in" -o "$T/o.zst" 2>/dev/null
  valgrind --tool=callgrind --callgrind-out-file="$T/d$l.$n" "$b" -q -f -d "$T/ref$l.zst" -o "$T/o.raw" 2>/dev/null
  cmp -s "$in" "$T/o.raw" || echo "ROUNDTRIP FAIL $n -$l"
  printf "c-%s %-6s %s\n" $l "$n" "$(total "$T/c$l.$n")"
  printf "d-%s %-6s %s\n" $l "$n" "$(total "$T/d$l.$n")"
done; done
echo "profiles: $T"
