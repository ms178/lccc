#!/usr/bin/env bash
# AUD-VABI-4: ONE-LANE FP vectors per spelling:
#   - ext_vector_type(1) (Clang-only spelling): argument AND return ride the
#     scalar carrier in the low half of %xmm0 (Clang receipt; GCC does not
#     accept the attribute). The fixtures below use this spelling.
#   - vector_size(4)/(8) (both spellings accepted): GCC 16.2.0 measured
#     MEMORY both ways (stack argument, sret return) — lccc follows the
#     oracle and matches it in cross-linked binaries in BOTH directions
#     (harness-cache-1 gcc-16.2.0: `f32x1 id(f32x1)` loads its argument from
#     8(%rsp) and returns through the hidden pointer).
# The two fixtures cover identity/arithmetic/chained forwarding and an
# exhaustive matrix (fptrs, volatile, memcpy, NaN/Inf/subnormal, integer
# one-lanes riding INTEGER), each at -O0/-O1/-O2.
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
