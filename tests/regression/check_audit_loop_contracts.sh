#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${CCC:-target/fastbuild/lccc}
src=tests/regression/audit_loop_contracts.c
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
for opt in -O1 -O2 -O3; do
    "$ccc" "$opt" "$src" -o "$td/check"
    "$td/check"
done
"$ccc" -O2 -S "$src" -o "$td/check.s"
python3 - "$td/check.s" <<'PY'
import re, sys
text = open(sys.argv[1]).read()
for fn in ('dw_lt', 'dw_le', 'dw_ne', 'dw_step2', 'dw_break'):
    body = re.search(r'^'+fn+r':\n(.*?)^\.size\s+'+fn+r'\b', text, re.M | re.S)
    if body is None:
        raise SystemExit('missing function: '+fn)
    if re.search(r'^\s*set[a-z]+\s', body[1], re.M):
        raise SystemExit('materialized bottom-test condition: '+fn)
print('bottom-tested branch contracts: PASS')
PY
# Correctness contracts for the new IVSR fix and quarantined FP candidate.
# Presence flags require env -u, not NAME=0, for the default arm.
for opt in -O0 -O1 -O2 -O3; do
    "$ccc" "$opt" tests/regression/ivsr_narrow_wrap.c -o "$td/wrap"
    "$td/wrap"
    for isa in x86-64 x86-64-v3; do
        env -u CCC_FP_EXTRACT_HOMES "$ccc" "$opt" -march="$isa" \
            tests/regression/slp_fp_extract_home.c -o "$td/fp"
        "$td/fp"
        CCC_FP_EXTRACT_HOMES=1 "$ccc" "$opt" -march="$isa" \
            tests/regression/slp_fp_extract_home.c -o "$td/fp"
        "$td/fp"
    done
done
for opt in -O1 -O2 -O3; do
    for test in va_arg_pack_len_inline fortify_open_va_pack; do
        "$ccc" "$opt" tests/regression/"$test".c -o "$td/vapack"
        "$td/vapack"
    done
done
