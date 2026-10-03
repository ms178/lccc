#!/usr/bin/env bash
# GVN cross-signedness load CSE: the I8->U8 forwarding must fire on the
# dispatch chain, and the optimized binary must agree with the GVN-off
# baseline (differential against sext/zext confusion).
set -eu

CCC=${CCC:-./target/fastbuild/lccc}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tmp="${TMPDIR:-/tmp}/lccc-gvn-xload.$$"
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
mkdir -p "$tmp"

CCC_DEBUG_GVN=1 "$CCC" -O2 "$dir/gvn_xsign_load_cse.c" \
    -o "$tmp/test" 2>"$tmp/compile.log"
# At least the p[0] U8 reloads (exit arm + later arms) forward from the I8
# sign-test load.
test "$(grep -c '^\[GVNDBG\] CSE xload .* (I8->U8)$' "$tmp/compile.log")" -ge 2

"$tmp/test" >"$tmp/run.log"
grep -Eq '^OK gvn_xsign_load_cse [0-9]+$' "$tmp/run.log"

# Differential: GVN off must produce identical output.
CCC_DISABLE_PASSES=gvn "$CCC" -O2 "$dir/gvn_xsign_load_cse.c" \
    -o "$tmp/test_base" 2>/dev/null
"$tmp/test_base" >"$tmp/run_base.log"
cmp -s "$tmp/run.log" "$tmp/run_base.log"
