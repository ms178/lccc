#!/usr/bin/env bash
# Cross-PR interaction red-team gate (session 574).
#
# WHY THIS EXISTS
# ---------------
# The per-feature gates pin each landed PR's own contract; nothing pinned
# the INTERACTIONS between them. This battery was built to red-team the
# last ten merged PRs as a stack (#557..#573) and it found two real
# defects the per-PR gates could not see:
#
#   * the FMA operand-negation peel rejected every multi-use Neg, so the
#     shared-negation shapes (one -b/-c read by two fma sites — the
#     phi-diamond body) kept two vxorpd + two vfmadd where GCC contracts
#     to two vfnmsub (fixed with the absorbability fixpoint);
#   * -mno-avx2 killed the entire VEX encoding (not just the 256-bit
#     class), declining the FMA fold into xorpd+xorpd+libm at the
#     AVX1+FMA target class (fixed with the dedicated avx2 denial).
#
# Both fixes are pinned here end-to-end: the battery's output is compared
# bit-exactly against GCC at matched march, in four configurations
# (default, -O3, the AVX1-class target, and the SSE2-no-FMA baseline),
# across every FP edge value (±0, ±inf, NaN, extremes) — the sign of
# zero and the sign of infinities included, which is what pins the FMA
# family selection. NaN SIGNS are compared under the documented latitude:
# a result NaN's sign is implementation-defined (C11 6.5p8 / Annex F),
# and with TWO NaN operands meeting, which NaN the hardware propagates
# depends on the compiler's operand placement — so canonical NaN bit
# patterns are normalised to a token on both sides before the diff.
# Everything else is bit-exact or the gate fails.
#
# Sections (cross_pr_redteam.c):
#   A  FMA families through copy brackets, calls and branches (#567 x
#      #571-#573)
#   B  FMA under if-conversion / select arms (#569 x #571)
#   C  the phi-diamond: shared negations across two fma sites — the
#      multi-use peel headline (#572)
#   D  FMA weighted dot over a .rodata table (#566 x #572)
#   E  guarded clz/ctz over promoted constant arrays (#569 x #566)
#   F  aliased-operand fma (the documented ordinary-path residual)
#   G  multi-use negation grammar corners (integer Neg, two-use shapes)
#   H  wide SLP FMA packs at all four widths (#572's P0 fix pinned)
#   J  in-place accumulator streams through the peephole store (#563)
#   K  everything stacked: range fusion + selects + brackets + packs
# cross_pr_redteam2.c adds the range-fusion domain corners (empty/point/
# INT_MIN domains, unsigned OR wrap, FP-NaN non-fusion), constant-array
# promotion under aliasing, copy-elim x peel interaction with calls, and
# the eight-accumulator regalloc pressure web.
#
# Usage:
#   tests/regression/check_cross_pr_redteam.sh
# Environment:
#   LCCC            compiler to test (default: target/fastbuild/lccc)
#   CCC_ORACLE_CC   reference compiler (default: cc, gcc or clang)
set -u -o pipefail

repo=$(cd "$(dirname "$0")/../.." && pwd)
CCC=${LCCC:-$repo/target/fastbuild/lccc}
if [ -x "$CCC" ]; then CCC=$(cd "$(dirname "$CCC")" && pwd)/$(basename "$CCC"); fi
ORACLE=${CCC_ORACLE_CC:-}
if [ -z "$ORACLE" ]; then
    for c in cc gcc clang; do
        if command -v "$c" >/dev/null 2>&1; then ORACLE=$c; break; fi
    done
fi
if [ -z "$ORACLE" ]; then
    echo "cross-pr-redteam: no reference compiler (cc/gcc/clang) found" >&2
    exit 2
fi

td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
fail=0
n_cfg=0

# Normalise canonical NaN encodings on both sides of the comparison:
# quiet double NaN 7ff8/fff8..0, quiet float NaN 7fc/ffc..0. Payloads
# beyond the quiet bit cannot occur here (all inputs are canonical).
normalise() {
    sed -E 's/\b(7|f)ff8[0-9a-f]{12}\b/NAN64/g; s/\b(7|f)fc[0-9a-f]{6}\b/NAN32/g' "$1"
}

run_cfg() { # run_cfg <label> <lccc-flags...>
    local label=$1; shift
    local src out
    n_cfg=$((n_cfg + 1))
    for src in cross_pr_redteam.c cross_pr_redteam2.c; do
        out=$td/${label}-$(basename "$src" .c)
        if ! "$CCC" "$@" -o "$out.bin" "$repo/tests/regression/$src" -lm 2>"$out.cerr"; then
            echo "FAIL: $label $src did not compile" >&2
            cat "$out.cerr" >&2
            fail=1
            continue
        fi
        if ! "$out.bin" > "$out.lccc" 2>&1; then
            echo "FAIL: $label $src exited non-zero" >&2
            fail=1
        fi
    done
}

cmp_cfg() { # cmp_cfg <label> <oracle-flags...>
    local label=$1; shift
    local src ref out
    for src in cross_pr_redteam.c cross_pr_redteam2.c; do
        ref=$td/${label}-$(basename "$src" .c).ref
        out=$td/${label}-$(basename "$src" .c)
        if ! "$ORACLE" -O2 "$@" -o "$td/ref.bin" "$repo/tests/regression/$src" -lm 2>/dev/null; then
            echo "FAIL: oracle could not compile $src ($*)" >&2
            fail=1
            continue
        fi
        if ! "$td/ref.bin" > "$ref" 2>&1; then
            echo "FAIL: oracle binary of $src exited non-zero" >&2
            fail=1
            continue
        fi
        if ! diff <(normalise "$ref") <(normalise "$out.lccc") > "$out.diff"; then
            echo "FAIL: $label $src differs from the oracle:" >&2
            head -20 "$out.diff" >&2
            fail=1
        fi
    done
}

# ---- 1. matched v3: the mainline contract (families, packs, brackets) ------
run_cfg v3 -O2 -march=x86-64-v3
cmp_cfg v3 -march=x86-64-v3

# ---- 2. -O3 on top: unrolling + the full pipeline over the same shapes ----
run_cfg O3 -O3 -march=x86-64-v3
cmp_cfg O3 -march=x86-64-v3

# ---- 3. the AVX1+FMA target class: VEX.128 stays, 256-bit goes ------------
# (the -mno-avx2 fix: the scalar FMA families must stay inline here)
run_cfg avx1 -O2 -march=x86-64-v3 -mno-avx2
cmp_cfg avx1 -march=x86-64-v3 -mno-avx2
# ymm denial is pinned by check_vectorize_isa_gate.sh; here pin that the
# AVX1-class build keeps the negated FMA families (no libm fallback):
"$CCC" -O2 -march=x86-64-v3 -mno-avx2 -S "$repo/tests/regression/cross_pr_redteam.c" \
    -o "$td/avx1.s" 2>/dev/null
if [ "$(grep -cE '\bvfn(madd|sub)[0-9]*p?s[dd]?\b' "$td/avx1.s" || true)" -eq 0 ]; then
    echo "FAIL: AVX1-class target lost the negated FMA families" >&2
    fail=1
fi
if [ "$(grep -c 'fma@PLT' "$td/avx1.s" || true)" -ne 0 ]; then
    echo "FAIL: AVX1-class target regressed to the fma libcall" >&2
    fail=1
fi

# ---- 4. the SSE2 baseline (no FMA at all): pure uncontracted parity -------
run_cfg base -O2 -march=x86-64
cmp_cfg base -march=x86-64

if [ "$fail" -eq 0 ]; then
    echo "cross-PR red-team gate: PASS ($n_cfg build configurations, 4 oracle comparisons x 2 batteries)"
else
    echo "cross-PR red-team gate: FAIL" >&2
fi
exit $fail
