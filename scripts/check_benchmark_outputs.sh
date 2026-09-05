#!/usr/bin/env bash
# Correctness gate over the benchmark corpus.
#
# WHY THIS EXISTS
#
# `tests/benchmark/programs/*.c` are real workload kernels -- SQLite varint,
# zlib-ng adler32, Expat scanning, glibc memcmp, Linux find_bit -- and they are
# the largest, most realistic programs in the tree. They were only ever
# compiled by `run_benchmarks.py`, which takes minutes because it times
# everything, so nobody ran them during ordinary development.
#
# That gap let a miscompile through: an induction-variable widening change
# passed all 563 regression tests and `sqlite_varint` silently printed
# `8e8824b0a241168` where GCC printed `deedcdd4edc1c0f1`. The regression corpus
# simply contains nothing with that loop shape.
#
# This script closes the gap. It is correctness-only -- compile, run, diff
# against the GCC oracle -- so it finishes in seconds and can be run on every
# change, unlike the timing harness.
#
# Usage:
#   scripts/check_benchmark_outputs.sh              # whole corpus
#   scripts/check_benchmark_outputs.sh sqlite       # substring filter
#
# Environment:
#   LCCC_BIN   compiler under test (default target/fastbuild/lccc)
#   GCC_BIN    reference compiler  (default gcc)
#   OPT        optimisation level to compare at (default: -O0 -O1 -O2 -O3)
set -u

REPO=${LCCC_REPO:-$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)}
LCCC_BIN=${LCCC_BIN:-$REPO/target/fastbuild/lccc}
GCC_BIN=${GCC_BIN:-gcc}
FILTER=${1:-}
LEVELS=${OPT:-"-O0 -O1 -O2 -O3"}

if [[ ! -x $LCCC_BIN ]]; then
    echo "error: $LCCC_BIN not found; build first" >&2
    exit 2
fi

# Match what run_benchmarks.py passes, so a failure here reproduces there.
GCC_INC=""
for d in /usr/lib/gcc/x86_64-linux-gnu/*/include; do
    [[ -d $d ]] && GCC_INC="-I$d"
done

WORK=$(mktemp -d -t lccc-benchcheck-XXXXXX)
trap 'rm -rf "$WORK"' EXIT

pass=0; fail=0; skip=0
declare -a FAILED=()

for src in "$REPO"/tests/benchmark/programs/*.c; do
    name=$(basename "$src" .c)
    [[ -n $FILTER && $name != *$FILTER* ]] && continue

    # LCCC at every level: an optimisation-level-specific miscompile is still a
    # miscompile, and -O0 vs -O3 disagreeing is the cheapest possible signal.
    #
    # The reference is rebuilt AT THE SAME LEVEL. Comparing lccc -O0 against
    # gcc -O2 is not a correctness test: `tce_sum` recurses ten million deep
    # and only survives because tail-call elimination turns it into a loop, so
    # an unoptimised build legitimately overflows the stack. Both compilers
    # must be asked the same question.
    for lev in $LEVELS; do
        ref_bin="$WORK/$name.ref$lev"
        if ! "$GCC_BIN" "$lev" "$src" -o "$ref_bin" -lm 2>/dev/null \
           && ! "$GCC_BIN" "$lev" "$src" -o "$ref_bin" 2>/dev/null; then
            skip=$((skip+1)); continue
        fi
        # STDOUT ONLY.  The benchmark programs log wall-clock timings to
        # stderr (they differ on every run and across compilers); the
        # byte-compared correctness signal is stdout.  Capturing 2>&1 here
        # flipped every timing-logging program's comparison into a coin
        # toss (i686_alu_chains failed 4/4 levels on identical checksums).
        ref_out=$(timeout 300 "$ref_bin" 2>/dev/null); ref_ec=$?
        if [[ $ref_ec -eq 124 ]]; then skip=$((skip+1)); continue; fi

        bin="$WORK/$name$lev"
        if ! "$LCCC_BIN" $GCC_INC "$lev" "$src" -o "$bin" 2>"$WORK/cc.err"; then
            echo "FAIL  $name $lev (build: $(head -2 "$WORK/cc.err" | tr '\n' ' '))"
            fail=$((fail+1)); FAILED+=("$name$lev:build"); continue
        fi
        out=$(timeout 300 "$bin" 2>/dev/null); ec=$?
        if [[ $ec -eq 124 ]]; then
            echo "FAIL  $name $lev (timeout)"
            fail=$((fail+1)); FAILED+=("$name$lev:timeout"); continue
        fi
        if [[ "$out|$ec" != "$ref_out|$ref_ec" ]]; then
            echo "FAIL  $name $lev (output differs from GCC)"
            echo "      gcc  : $(printf '%s' "$ref_out" | head -2)"
            echo "      lccc : $(printf '%s' "$out" | head -2)"
            fail=$((fail+1)); FAILED+=("$name$lev:oracle"); continue
        fi
        pass=$((pass+1))
    done
done

echo
echo "================================================================"
echo "benchmark output gate: PASS=$pass FAIL=$fail SKIP=$skip"
if [[ ${#FAILED[@]} -gt 0 ]]; then
    printf 'failed: %s\n' "${FAILED[*]}"
fi
echo "================================================================"
[[ $fail -eq 0 ]]
