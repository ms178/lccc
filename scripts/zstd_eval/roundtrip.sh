#!/bin/sh
# zstd round trip with cross-decoding: A compresses at levels 1/3/9/19, then
# both A and B decompress and the bytes must match the original.
# usage: roundtrip.sh <zstd-under-test> <reference-zstd> <file>...
A=$1; B=$2; shift 2; T=${TMPDIR:-/tmp}/zrt.$$; mkdir -p "$T"; pass=0; fail=0
for f in "$@"; do for l in 1 3 9 19; do
  "$A" -q -f -$l "$f" -o "$T/c.zst" || { echo "FAIL compress $f -$l"; fail=$((fail+1)); continue; }
  for d in "$A" "$B"; do
    if "$d" -q -f -d "$T/c.zst" -o "$T/o" && cmp -s "$f" "$T/o"; then pass=$((pass+1)); else echo "FAIL $f -$l dec=$d"; fail=$((fail+1)); fi
  done
done; done
rm -rf "$T"; echo "roundtrip pass=$pass fail=$fail"; [ $fail -eq 0 ]
