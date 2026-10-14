#!/usr/bin/env bash
# A14-30: a late-emitted constant-offset GEP must not feed an unwritten
# MachInst pointer home. Keep the original licensed two-TU extraction and
# compare the ENTIRE generated table, not just compression round trips.
set -euo pipefail
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
CCC=${CCC:-$ROOT/target/fastbuild/lccc}
DATA="$ROOT/engineering/evidence/2026-10-14-audit/zlib-maketrees"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
for opt in O0 O1 O2 O3 Os Oz; do
    timeout 60 "$CCC" "-$opt" "$DATA/maketrees-small.c" \
        "$DATA/gencodes-small.c" -o "$work/trees"
    timeout 10 "$work/trees" > "$work/actual"
    cmp "$DATA/expected.txt" "$work/actual"
    echo "PASS zlib-ng maketrees $opt (complete table)"
done
# These controls isolate the enabling transformations from the actual
# backend defect. None is needed by the fixed default pipeline.
for control in CCC_NO_IV_WIDEN CCC_NO_MACHINST LCCC_NO_PEEPHOLE; do
    env "$control=1" timeout 60 "$CCC" -O2 "$DATA/maketrees-small.c" \
        "$DATA/gencodes-small.c" -o "$work/trees"
    timeout 10 "$work/trees" > "$work/actual"
    cmp "$DATA/expected.txt" "$work/actual"
    echo "PASS zlib-ng maketrees O2 $control (complete table)"
done
