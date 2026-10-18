#!/usr/bin/env bash
# Regression: one-lane ext_vector_type FP/int carrier correctness.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$root/target/fastbuild/lccc}
[ -x "$ccc" ] || { echo "FAIL: $ccc not built" >&2; exit 1; }
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
for opt in -O0 -O1 -O2; do
  "$ccc" "$root/tests/regression/ext_vector_lane1_abi.c" -o "$tmp/t" $opt
  out=$("$tmp/t")
  [ "$out" = "OK ext_vector_lane1_abi" ] || { echo "FAIL ext_vector_lane1_abi ($opt): $out"; exit 1; }
done
echo "PASS check_ext_vector_lane1_abi"
