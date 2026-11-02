#!/usr/bin/env bash
# De Morgan branch split: the And-direct site (fill) and the Or-cast
# passthru site (scan) must both fire, and the split binary must agree
# with the unsplit baseline (CCC_NO_DEMORGAN=1) on the checksum.
set -eu

CCC=${CCC:-./target/fastbuild/lccc}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tmp="${TMPDIR:-/tmp}/lccc-demorgan-split.$$"
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
mkdir -p "$tmp"

LCCC_DEBUG_DEMORGAN=1 "$CCC" -O2 "$dir/demorgan_branch_split.c" \
    -o "$tmp/test" 2>"$tmp/compile.log"
# Exactly one And fire and one Or fire.
test "$(grep -c '^\[demorgan\] fire andor=[0-9]* is_and=true$' "$tmp/compile.log")" -eq 1
test "$(grep -c '^\[demorgan\] fire andor=[0-9]* is_and=false$' "$tmp/compile.log")" -eq 1

"$tmp/test" >"$tmp/run.log"
grep -Eq '^OK demorgan_branch_split [0-9]+$' "$tmp/run.log"

# Differential: split vs unsplit must produce identical output.
CCC_NO_DEMORGAN=1 "$CCC" -O2 "$dir/demorgan_branch_split.c" \
    -o "$tmp/test_base" 2>/dev/null
"$tmp/test_base" >"$tmp/run_base.log"
cmp -s "$tmp/run.log" "$tmp/run_base.log"
