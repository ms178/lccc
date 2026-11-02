#!/usr/bin/env bash
# Regression: narrow one-lane ext_vector arithmetic (add/sub/mul/neg) and
# signedness preservation across -O0/-O2 and A/B small-slot configs.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$root/target/fastbuild/lccc}
[ -x "$ccc" ] || { echo "FAIL: $ccc not built" >&2; exit 1; }
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
src=tests/regression/ext_vector_narrow_arith.c
for opt in -O0 -O1 -O2; do
  for slots in 0 1; do
    if [ "$slots" = "1" ]; then
      CCC_NO_SMALL_SLOTS=1 "$ccc" "$root/$src" -o "$tmp/t" $opt
    else
      "$ccc" "$root/$src" -o "$tmp/t" $opt
    fi
    out=$("$tmp/t")
    [ "$out" = "OK ext_vector_narrow_arith" ] || { echo "FAIL ext_vector_narrow_arith ($opt small_slots=$slots): $out"; exit 1; }
  done
done
echo "PASS check_ext_vector_narrow_arith"
