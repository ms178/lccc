#!/usr/bin/env bash
# AUD-VABI-4: ONE-LANE FP vectors per spelling, as the oracles measure them:
#   - ext_vector_type(1) double (Clang-only spelling): returned in the low
#     half of %xmm0 (Clang 23.1.3 receipt: `movsd 8(%rsp), %xmm0; ret`).
#   - ext_vector_type(1) float: returned through the INTEGER eightbyte —
#     %eax/%rax carries the bit pattern, like u32x1/u64x1. Measured by the
#     pinned Clang 23.1.3 ext_vector_abi_grid (exact expectations, all four
#     link directions); the "float twin of f64x1" analogy that once pinned
#     an SSE arm here was GCC-16-only evidence, and GCC 16 does not accept
#     the attribute at all, so it is no oracle for this spelling.
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
echo 'one-lane FP vector ABI: PASS (ext f64x1 in %xmm0, ext f32x1 INTEGER bits per Clang 23.1.3 grid, integer one-lanes in GPRs, vector_size one-lanes MEMORY per GCC 16.2.0, at -O0/-O1/-O2)'
