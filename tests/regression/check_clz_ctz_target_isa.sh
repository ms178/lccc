#!/usr/bin/env bash
# Emission gate for the bit-manipulation ISA contract:
#   - default (no -march)                          -> lzcnt/tzcnt AND popcnt:
#     the project code-generation baseline is x86-64-v3, whose SIMD ceiling
#     (AVX2) already makes every default binary v3-only; the integer half
#     (BMI1/BMI2/LZCNT/POPCNT/MOVBE) is granted with it (one ISA, one
#     hardware floor — the pre-wiring split granted AVX2 but not LZCNT).
#   - -march=x86-64 / -march=x86-64-v1             -> NO lzcnt/tzcnt/popcnt
#     (F3-prefixed LZCNT/TZCNT decode as BSR/BSF on non-ABM CPUs and
#     silently return inverted counts — the exact bug that aborted the
#     lccc-built preboot ZSTD decoder on QEMU's qemu64); the fallback must
#     be the BSR/BSF + explicit-zero-fixup form.
#   - default plus -mno-lzcnt                      -> BSR/BSF fallback:
#     the explicit denial wins over the v3 default grant.
#   - -march=x86-64-v2                             -> popcnt only
#   - -mlzcnt -mpopcnt / -march=x86-64-v3          -> all three
# The runtime semantics themselves are covered differentially by
# clz_ctz_popcount_target_isa.c; this script pins the INSTRUCTION choice.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
src=$repo/tests/regression/clz_ctz_popcount_target_isa.c
inc=${GCC_INC:-$(gcc -print-file-name=include)}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

emit() { # emit <label> <flags...>
    local label=$1; shift
    "$ccc" -O2 "$@" -I"$inc" -S "$src" -o "$tmp/$label.s"
}

count() { grep -cE '\b(lzcnt|tzcnt)[bwlq]?\b' "$1" || true; }
countpop() { grep -cE '\bpopcnt[bwlq]?\b' "$1" || true; }
countbsr() { grep -cE '\bbsr[bwlq]?\b|\bbsf[bwlq]?\b' "$1" || true; }

fail=0
check() { # check <desc> <actual> <expected>
    local desc=$1
    if [[ "$2" != "$3" ]]; then
        echo "FAIL: $desc — got '$2', want '$3'" >&2
        fail=1
    fi
}

# Default: the v3 grant — LZCNT/TZCNT and POPCNT are both on.
emit default
d=$tmp/default.s
[[ "$(count $d)" -gt 0 ]] || { echo "FAIL: default did not enable lzcnt/tzcnt (v3 baseline)" >&2; fail=1; }
[[ "$(countpop $d)" -gt 0 ]] || { echo "FAIL: default did not enable popcnt (v2 ⊂ v3)" >&2; fail=1; }

# Explicit-denial arm: the default grant must fold under -mno-lzcnt.
emit nolzcnt -mno-lzcnt
d=$tmp/nolzcnt.s
check "default -mno-lzcnt lzcnt/tzcnt emission" "$(count $d)" 0
if [[ "$(countbsr $d)" -eq 0 ]]; then
    echo "FAIL: -mno-lzcnt produced neither lzcnt nor bsr/bsf — fallback missing" >&2
    fail=1
fi

# Explicit baseline profiles keep the BSR/BSF + zero-fixup contract.
for cfg in "march-x86-64" "march-v1"; do
    case $cfg in
        march-x86-64) emit march-x86-64 -march=x86-64 ;;
        march-v1)     emit march-v1 -march=x86-64-v1 ;;
    esac
    d=$tmp/$cfg.s
    check "$cfg lzcnt/tzcnt emission" "$(count $d)" 0
    check "$cfg popcnt emission" "$(countpop $d)" 0
    # the baseline fallback must actually be BSR/BSF-based, not nothing
    if [[ "$(countbsr $d)" -eq 0 ]]; then
        echo "FAIL: $cfg produced neither lzcnt nor bsr/bsf — lowering missing" >&2
        fail=1
    fi
done

emit v2 -march=x86-64-v2
check "v2 lzcnt/tzcnt emission" "$(count $tmp/v2.s)" 0
[[ "$(countpop $tmp/v2.s)" -gt 0 ]] || { echo "FAIL: v2 did not enable popcnt" >&2; fail=1; }

emit v3 -march=x86-64-v3
[[ "$(count $tmp/v3.s)" -gt 0 ]] || { echo "FAIL: v3 did not enable lzcnt" >&2; fail=1; }
[[ "$(countpop $tmp/v3.s)" -gt 0 ]] || { echo "FAIL: v3 did not enable popcnt" >&2; fail=1; }

emit explicit -mlzcnt -mpopcnt
[[ "$(count $tmp/explicit.s)" -gt 0 ]] || { echo "FAIL: -mlzcnt ignored" >&2; fail=1; }
[[ "$(countpop $tmp/explicit.s)" -gt 0 ]] || { echo "FAIL: -mpopcnt ignored" >&2; fail=1; }

if [[ $fail -ne 0 ]]; then exit 1; fi
echo "clz/ctz/popcount ISA emission gates: PASS"
