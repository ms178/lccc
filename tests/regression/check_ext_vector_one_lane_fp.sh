#!/usr/bin/env bash
# AUD-VABI-4: one-lane FP vectors (f32x1/f64x1) return in the low half of
# %xmm0 like scalar f32/f64 — the same-compiler shape GCC 16 and Clang 19
# emit.  The two fixtures cover identity/arithmetic/chained forwarding and an
# exhaustive matrix (fptrs, volatile, memcpy, NaN/Inf/subnormal, integer
# one-lanes riding INTEGER), each at -O0/-O1/-O2.
# Oracle note: system GCC 12 predates the psABI small-vector update and
# passes vector_size(4/8) FP shapes through MEMORY, so the cross legs here
# are lccc-vs-lccc round trips with values pinned by the fixtures; the
# GCC-16 CI lanes are the cross-ABI receipts.
set -euo pipefail
root=$(cd -- "$(dirname -- "$0")/../.." && pwd)
CCC=${CCC:-"$root/target/fastbuild/lccc"}
[[ -x "$CCC" ]] || { echo "missing LCCC compiler: $CCC" >&2; exit 1; }
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

for opt in -O0 -O1 -O2; do
    for fx in ext_vector_one_lane_fp ext_vector_one_lane_fp_full; do
        "$CCC" "$opt" "$root/tests/regression/$fx.c" -o "$work/${fx}-${opt#-}"
        # The exhaustive fixture reports on stderr (its FAIL macros write
        # there), so merge streams before the exact-line check.
        "$work/${fx}-${opt#-}" >"$work/${fx}-${opt#-}.out" 2>&1
        grep -qx 'fails=0' "$work/${fx}-${opt#-}.out" || {
            echo "FAIL: $fx at $opt:" >&2
            cat "$work/${fx}-${opt#-}.out" >&2
            exit 1
        }
    done
done
echo 'one-lane FP vector ABI: PASS (f32x1/f64x1 in %xmm0, integer one-lanes in GPRs, at -O0/-O1/-O2)'
