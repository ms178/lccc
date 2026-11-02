#!/usr/bin/env bash
# Sparse masked updates must remain correct even on strongly biased inputs.
# The Rust detector tests check profitability; this checks executable semantics,
# including allocation without phi coalescing and without hot-web promotion.
set -euo pipefail
here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$here/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
. "$here/lib/gcc_oracle.sh"
require_gcc_oracle check_sparse_mask_updates
work=$(mktemp -d "${TMPDIR:-/tmp}/lccc-sparse-mask.XXXXXX")
trap 'rm -rf "$work"' EXIT
"$GCC_ORACLE_BIN" -O0 "$here/sparse_mask_updates.c" -o "$work/reference"
timeout 30 "$work/reference" > "$work/expected"
for opt in -O1 -O2 -O3; do
    for knob in default CCC_NO_HOT_WEB_STEAL CCC_NO_PHI_COALESCE; do
        if [[ $knob == default ]]; then
            "$CCC" "$opt" -march=x86-64-v3 "$here/sparse_mask_updates.c" -o "$work/candidate"
        else
            env "$knob=1" "$CCC" "$opt" -march=x86-64-v3 "$here/sparse_mask_updates.c" -o "$work/candidate"
        fi
        timeout 30 "$work/candidate" > "$work/actual"
        diff -u "$work/expected" "$work/actual"
    done
done
printf 'PASS: sparse masked updates, three optimization levels and three allocator policies\n'
