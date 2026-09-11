#!/usr/bin/env bash
# AB-14 follow-up: adjacent exact F128 load/store bypasses f64 approximation.
set -euo pipefail
CCC=${CCC:-./target/release/lccc}; t=$(mktemp -d); trap 'rm -rf "$t"' EXIT
src=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)/f128_param_home_order.c
"$CCC" -O2 "$src" -o "$t/run"; "$t/run"
"$CCC" -O2 -S "$src" -o "$t/t.s"
body=$(sed -n '/^wr:/,/^\.size wr/p' "$t/t.s")
grep -q 'movdqu' <<<"$body"
! grep -q 'fstpl' <<<"$body"
