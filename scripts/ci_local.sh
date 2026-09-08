#!/usr/bin/env bash
# Run every gate the GitHub `test` and `clippy` jobs run, in the same order,
# with the same environment, against the local tree.
#
# WHY THIS EXISTS
# ---------------
# The regression corpus is not the CI gate; it is ONE of nine.  A change can
# pass `run_regression.py` (720+ tests) and `cargo test` (2000+ unit tests)
# and still be a miscompile that only the differential correctness oracle,
# the benchmark-output oracle, or the emitted-assembly contract checks see.
# That is not hypothetical: the all-homed VEX compare fast path shipped with
# `(eq || !invert)` where it needed `!invert`, which silently turned every
# AVX2 `!=` lane mask into `==`.  Every regression test passed.  The
# differential gate failed.
#
# Running "the tests I remember" is therefore not a validation strategy.
# Run this script before every push; it is the contract.
#
#   ./scripts/ci_local.sh              # everything
#   ./scripts/ci_local.sh --fast       # skip the two slowest oracles
#   ./scripts/ci_local.sh --only NAME  # a single gate, substring match
#
# Exit status is non-zero if ANY gate fails, and every failure is repeated in
# the summary at the end so a long log cannot hide one.
set -u -o pipefail

cd "$(dirname "$0")/.."
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
export PATH="$CARGO_HOME/bin:$PATH"

FAST=0
ONLY=""
for arg in "$@"; do
    case "$arg" in
        --fast) FAST=1 ;;
        --only) ONLY="__NEXT__" ;;
        *) if [ "$ONLY" = "__NEXT__" ]; then ONLY="$arg"; else
               echo "unknown argument: $arg" >&2; exit 2
           fi ;;
    esac
done

LCCC=target/fastbuild/lccc
FAILED=()
PASSED=0
SKIPPED=0

hr() { printf '─%.0s' {1..72}; echo; }

# gate NAME SLOW COMMAND...
gate() {
    local name="$1"; shift
    local slow="$1"; shift
    if [ -n "$ONLY" ] && [[ "$name" != *"$ONLY"* ]]; then
        return 0
    fi
    if [ "$FAST" = "1" ] && [ "$slow" = "slow" ]; then
        echo "SKIP  $name (--fast)"
        SKIPPED=$((SKIPPED + 1))
        return 0
    fi
    hr
    echo "GATE  $name"
    hr
    local start
    start=$(date +%s)
    if "$@"; then
        echo "PASS  $name  ($(($(date +%s) - start))s)"
        PASSED=$((PASSED + 1))
    else
        echo "FAIL  $name  ($(($(date +%s) - start))s)"
        FAILED+=("$name")
    fi
}

# ---------------------------------------------------------------- build ----
gate "build" fast ./scripts/build_lccc_fast.sh

if [ ! -x "$LCCC" ]; then
    echo "FATAL: $LCCC was not produced; every later gate would be vacuous." >&2
    exit 1
fi

# --------------------------------------------------------------- job:test --
gate "rust-toolchain-selector" fast \
    bash tests/regression/check_rust_toolchain_selector.sh

gate "cargo-test" slow \
    cargo test --profile fastbuild --all-targets --locked -j 2

# CCC_VALIDATE_SSA is what CI sets; without it the corpus does not verify SSA
# form after every pass and a malformed-IR bug can hide behind a correct
# result.
gate "regression-corpus-ssa" slow \
    env CCC_VALIDATE_SSA=1 python3 tests/regression/run_regression.py \
        --lccc "$LCCC" -j 2 --json regression-results.json

gate "benchmark-output-oracle" slow \
    bash scripts/check_benchmark_outputs.sh

gate "differential-correctness-oracle" fast \
    env LCCC_BIN="$LCCC" python3 tests/correctness/run_correctness.py

gate "loop-alignment-contract" fast \
    python3 scripts/check_loop_alignment.py --lccc "$LCCC"

gate "fuzz-engine-wiring" fast \
    python3 scripts/fuzz_diff.py --check-engines

gate "differential-fuzz-smoke" fast \
    python3 scripts/fuzz_diff.py --engine synthetic --count 4 \
        --lccc "$LCCC" --refs gcc --opts=-O0,-O2 --seed 20260906

gate "strict-computed-recip-codegen" fast \
    bash tests/regression/check_strict_computed_recip_codegen.sh

gate "machinst-window-alloc" fast \
    bash tests/regression/check_machinst_window_alloc_wide_copy.sh

if [ -x target/fastbuild/lccc-ld ]; then
    gate "linker-fuzz" fast env \
        LCCC_LD="$PWD/target/fastbuild/lccc-ld" FUZZ_N=128 FUZZ_SEED=20260906 \
        python3 tests/linker/fuzz_ld.py
    gate "linker-elf-grammar-fuzz" fast \
        python3 tests/linker/fuzz_elf_grammar.py \
            --lccc "$PWD/target/fastbuild/lccc-ld" --iters 64 --seed 20260906
else
    echo "SKIP  linker fuzz (target/fastbuild/lccc-ld not built)"
    SKIPPED=$((SKIPPED + 2))
fi

# ------------------------------------------------------------- job:clippy --
gate "rustfmt" fast cargo fmt --all -- --check
gate "clippy" slow \
    cargo clippy --all-targets --profile fastbuild --locked -j 2 -- -D warnings

# ------------------------------------------------------------------ done ---
hr
echo "SUMMARY: ${PASSED} passed, ${#FAILED[@]} failed, ${SKIPPED} skipped"
if [ ${#FAILED[@]} -gt 0 ]; then
    for f in "${FAILED[@]}"; do echo "  FAILED: $f"; done
    exit 1
fi
echo "ALL GATES GREEN"
