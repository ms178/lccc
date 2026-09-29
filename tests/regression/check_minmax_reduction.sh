#!/usr/bin/env bash
# ============================================================================
# check_minmax_reduction.sh — MINMAX-1: integer min/max reductions vectorize on
# x86-64 AVX2, and every shape the transform must refuse stays scalar-correct.
#
# WHY A GATE AND NOT JUST A BENCHMARK
# -----------------------------------
# The reduction is a SELECT after if-conversion (`if (a[i] > mx) mx = a[i]`),
# so it only exists after the pass pipeline's if_convert phase — which runs
# AFTER the main vectorizer.  The late rerun's shared dispatch table was
# wired for AArch64 only; x86-64 was served by a separate, divergent
# `vectorize_function_late` twin.
#
# CORRECTION (an earlier version of this comment claimed the vpmaxsd
# machinery "was dead code on x86"). That is false. The twin reran the
# vectorizer on x86-64 with the same analysis flags; what it lacked was the
# shared entry's diamond/if-chain PRE-CONVERSION, which is what creates the
# Select a min/max reduction needs. The win comes from that pre-conversion
# plus the admission fixes — not from "opening" a rerun that was already
# open. Do not reintroduce the "dead code" phrasing.
# The failure mode of "fixing" that is not a slow kernel, it is a WRONG one,
# so this gate pins four contracts separately:
#
#   1. CORRECTNESS (differential, randomized). `minmax_harness.c` compares
#      lccc-compiled kernels against scalar references compiled by the GCC
#      oracle, over every length 1..80 (both sides of every vector-width and
#      remainder boundary), four input distributions (narrow, full-range,
#      INT_MIN/INT_MAX only, heavy duplicates), interior and tail extremes,
#      plus large lengths.  Both the whole program and the kernels alone are
#      compiled by lccc, because a miscompile can live in either.
#
#   2. THE MECHANISM FIRES. `minmax_kernels.c` must lower to 256-bit
#      `vpmin*`/`vpmax*` steady-state loops; `check_minmax_shapes.py` asserts
#      the vector width and the per-byte cost, so a silent fall back to scalar
#      is a failure, not a pass.
#
#   3. THE REFUSALS STAY REFUSED. Extra accumulators in the same loop (sum
#      beside the extreme, or min AND max), unsigned compares, 16-bit
#      elements, float elements and guarded (`continue`) updates are all
#      shapes this transform does not model.  They must stay scalar — and
#      still be correct.  Refusing is load-bearing: modelling a multi-
#      accumulator loop with a single-accumulator pattern produced sum == 0
#      for every n below the vector width, which is exactly the miscompile
#      contract 1 exists to catch.
#
#   4. NO ARCHITECTURE IS LEFT BEHIND. i686 and the SSE2 baseline have no
#      horizontal-min lowering; the corpus must still agree with the oracle
#      there (scalar, correct) rather than ICE or miscompile.
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
GCC=${GCC:-gcc}
[[ -x $CCC ]] || { echo "check_minmax_reduction: lccc not found at $CCC" >&2; exit 1; }
command -v "$GCC" >/dev/null 2>&1 || { echo "check_minmax_reduction: no gcc oracle" >&2; exit 1; }

# NB: the harness and the refused-shapes main are NOT standalone programs --
# they are drivers whose kernels live in minmax_kernels.c / minmax_shapes.c.
# They deliberately sit in the minmax_shapes/ subdirectory because
# tests/regression/run_regression.py globs tests/regression/*.c
# non-recursively and compiles every hit as a self-contained program.
harness=$here/minmax_shapes/minmax_harness.c
kernels=$here/minmax_shapes/minmax_kernels.c
shapes=$here/minmax_shapes/minmax_shapes.c
work=$(mktemp -d "${TMPDIR:-/tmp}/lccc-minmax.XXXXXX")
trap 'rm -rf "$work"' EXIT
fail=0
note() { printf '  %s\n' "$*"; }
bad() { printf '  FAIL: %s\n' "$*" >&2; fail=1; }

# ---------------------------------------------------------------- contract 1
note "contract 1: randomized differential correctness vs the GCC oracle"
for arch in x86-64 x86-64-v3; do
    # Reference: scalar C compiled by the oracle, kernels + harness together.
    "$GCC" -O2 "-march=$arch" "$harness" "$kernels" -o "$work/ref-$arch" \
        || bad "$arch: gcc oracle failed to build"
    timeout 120 "$work/ref-$arch" > "$work/expected-$arch" \
        || bad "$arch: gcc oracle exited non-zero"
    for opt in -O0 -O1 -O2 -O3 -Os; do
        # (a) whole program compiled by lccc
        if ! "$CCC" $opt "-march=$arch" "$harness" "$kernels" -o "$work/lccc-$arch" 2>"$work/cc.log"; then
            bad "$arch $opt: lccc failed to build the harness"
            sed 's/^/    /' "$work/cc.log" >&2 || true
            continue
        fi
        if timeout 120 "$work/lccc-$arch" > "$work/got-$arch" 2>&1; then
            cmp -s "$work/expected-$arch" "$work/got-$arch" \
                || bad "$arch $opt: harness output differs from the oracle"
        else
            bad "$arch $opt: lccc-built harness exited non-zero (crash/miscompile)"
            head -5 "$work/got-$arch" >&2 || true
        fi
        # (b) kernels compiled by lccc, reference harness by the oracle:
        #     isolates the vectorized kernels from harness-side codegen.
        "$CCC" $opt "-march=$arch" -c "$kernels" -o "$work/kern.o" 2>/dev/null \
            || { bad "$arch $opt: lccc failed to compile the kernels"; continue; }
        "$GCC" -O2 "-march=$arch" "$harness" "$work/kern.o" -o "$work/mixed-$arch" \
            || { bad "$arch $opt: mixed link failed"; continue; }
        if timeout 120 "$work/mixed-$arch" > "$work/got2-$arch" 2>&1; then
            cmp -s "$work/expected-$arch" "$work/got2-$arch" \
                || bad "$arch $opt: vectorized kernels disagree with the oracle"
        else
            bad "$arch $opt: vectorized kernels exited non-zero"
            head -5 "$work/got2-$arch" >&2 || true
        fi
    done
done

# ---------------------------------------------------------------- contract 2
note "contract 2: the min/max steady-state loop is 256-bit (AVX2)"
python3 "$root/scripts/check_minmax_shapes.py" --ccc "$CCC" --source "$shapes" \
    || bad "min/max shapes did not lower to packed min/max"

# ---------------------------------------------------------------- contract 3
note "contract 3: the refused shapes stay scalar and stay correct"
"$GCC" -O2 -march=x86-64-v3 "$shapes" "$here/minmax_shapes/minmax_refused_main.c" -o "$work/refused-ref" \
    || bad "gcc oracle failed to build the refused-shape program"
timeout 120 "$work/refused-ref" > "$work/refused-expected" \
    || bad "refused-shape oracle exited non-zero"
for opt in -O1 -O2 -O3; do
    for arch in x86-64 x86-64-v3; do
        "$CCC" $opt "-march=$arch" "$shapes" "$here/minmax_shapes/minmax_refused_main.c" \
            -o "$work/refused-lccc" 2>/dev/null \
            || { bad "$arch $opt: refused-shape program failed to build"; continue; }
        timeout 120 "$work/refused-lccc" > "$work/refused-got" 2>&1 \
            || { bad "$arch $opt: refused-shape program exited non-zero"; continue; }
        cmp -s "$work/refused-expected" "$work/refused-got" \
            || bad "$arch $opt: refused-shape output differs from the oracle"
    done
done

# ---------------------------------------------------------------- contract 4
note "contract 4: i686 (no AVX2 min/max) still agrees with the oracle"
if "$CCC" -O2 -m32 "$harness" "$kernels" -o "$work/i686" 2>/dev/null; then
    if "$GCC" -O2 -m32 "$harness" "$kernels" -o "$work/i686-ref" 2>/dev/null; then
        timeout 180 "$work/i686-ref" > "$work/i686-expected" || bad "i686 oracle failed"
        if timeout 180 "$work/i686" > "$work/i686-got" 2>&1; then
            cmp -s "$work/i686-expected" "$work/i686-got" \
                || bad "i686: output differs from the oracle"
        else
            bad "i686: lccc-built program exited non-zero"
        fi
    else
        note "  (skipped: no 32-bit gcc multilib available)"
    fi
else
    note "  (skipped: lccc cannot target i686 in this configuration)"
fi

if [[ $fail -eq 0 ]]; then
    echo "check_minmax_reduction: PASS"
else
    echo "check_minmax_reduction: FAIL" >&2
fi
exit $fail
