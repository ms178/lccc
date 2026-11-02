#!/usr/bin/env bash
# AArch64 FP SSA values with stack homes must load directly into sN/dN. Routing
# a spill through x0 adds an instruction and a cross-domain dependency
# (`ldr x0` + `fmov dN,x0`) while preserving exactly the same bits.
set -euo pipefail
CCC_ARM=${CCC_ARM:-./target/fastbuild/lccc-arm}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

# Keep more independent FP values live than the allocator's FP pool. Building
# the source here keeps the checked-in regression compact while deterministically
# forcing scalar F64 spill homes.
{
    echo 'double fp_spill_reload(const double *p) {'
    for i in $(seq 0 31); do
        a=$((2*i)); b=$((a+1)); c=$((i+1))
        printf '  double x%d = p[%d] * p[%d] + %d.0;\n' "$i" "$a" "$b" "$c"
    done
    printf '  return x0'
    for i in $(seq 1 31); do printf ' + x%d' "$i"; done
    echo ';'
    echo '}'
} >"$td/probe.c"

"$CCC_ARM" -O2 -S -o "$td/probe.s" "$td/probe.c"

if ! grep -Eq '^[[:space:]]+ldr d[0-9]+, \[sp, #[0-9]+\]$' "$td/probe.s"; then
    echo 'no direct F64 spill reload was generated' >&2
    cat "$td/probe.s" >&2
    exit 1
fi

# Reject the exact old two-instruction sequence, allowing unrelated uses of x0.
if awk '
    /^[[:space:]]+ldr x0, \[sp, #[0-9]+\]$/ { previous_spill = 1; next }
    previous_spill && /^[[:space:]]+fmov d[0-9]+, x0$/ { bad = 1 }
    { previous_spill = 0 }
    END { exit bad ? 0 : 1 }
' "$td/probe.s"; then
    echo 'an F64 spill reload still bounces through x0' >&2
    cat "$td/probe.s" >&2
    exit 1
fi
