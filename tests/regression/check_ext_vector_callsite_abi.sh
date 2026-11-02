#!/usr/bin/env bash
# Regression: caller-side marshalling of one-lane ext_vector arguments places
# them in the correct register class and width at -O0 and -O2.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$root/target/fastbuild/lccc}
[ -x "$ccc" ] || { echo "FAIL: $ccc not built" >&2; exit 1; }
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
src=tests/regression/ext_vector_callsite_abi.c
for opt in -O0 -O1 -O2; do
  "$ccc" "$root/$src" -o "$tmp/t" $opt
  out=$("$tmp/t")
  [ "$out" = "OK ext_vector_callsite_abi" ] || { echo "FAIL ext_vector_callsite_abi ($opt): $out"; exit 1; }
done
echo "PASS check_ext_vector_callsite_abi"
