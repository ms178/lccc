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
#   ./scripts/ci_local.sh              # everything (test + bench + clippy)
#   ./scripts/ci_local.sh --fast       # skip the two slowest oracles
#   ./scripts/ci_local.sh --only NAME  # a single gate, substring match
#
# Environment:
#   LCCC_TEST_REPEATS=N   run the unit-test suite N times (default 1). N > 1
#                         surfaces tests that depend on process-global state.
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

# The unit-test gate is what GitHub's required check runs on every PR, so it
# belongs in the fast set: a red PR has to be reproducible with --fast.
#
# The suite runs its tests on parallel threads, so a test that touches
# process-global state -- the environment, a static -- passes or fails
# depending on what its neighbours are doing. A single run hides that class of
# bug: PR #471 was red with a ~1-in-10 failure while every local single-shot
# run was green. Repeating the suite is the only way to see it, and it is
# cheap once the test binaries are built. Set LCCC_TEST_REPEATS to raise the
# count (CI itself runs it once).
#
# Memory-constrained hosts: the monolithic lib-test compile with the
# fastbuild profile's line-tables debuginfo needs ~3 GB of resident rustc;
# on a 4 GB sandbox the OOM killer SIGKILLs it and the gate reports a
# compile failure that has nothing to do with the code under test. The
# test binary's debuginfo contributes nothing the assertions read --
# panic messages carry their own source spans -- and cargo's -j only
# serialises the COMPILE (the test harness's own parallelism is
# --test-threads, untouched), so on hosts with less than 6 GB of RAM
# the test compile drops the debuginfo AND builds one rustc at a time
# (CARGO_PROFILE_FASTBUILD_DEBUG=0, -j 1; set the env var explicitly to
# override the heuristic, including back to the profile default with
# CARGO_PROFILE_FASTBUILD_DEBUG=line-tables-only).
cargo_test_repeated() {
    local flags="" n i dbg jobs
    # Reuse the flags the build gate resolved, or cargo rebuilds everything.
    [ -r target/lccc-rustflags ] && flags="$(cat target/lccc-rustflags)"
    dbg="${CARGO_PROFILE_FASTBUILD_DEBUG:-}"
    incr="${CARGO_INCREMENTAL:-}"
    jobs=2
    if [ -z "$dbg" ]; then
        local total_mb
        total_mb=$(free -m 2>/dev/null | awk '/^Mem:/{print $2}')
        if [ -n "$total_mb" ] && [ "$total_mb" -lt 6000 ]; then
            dbg=0
            jobs=1
        fi
    fi
    # The debuginfo drop alone is not always enough: rustc's incremental
    # session state for the monolithic lib-test compile adds roughly a
    # gigabyte of resident memory on this crate, and on a 4 GB host with
    # a cold page cache the OOM killer still SIGKILLs a compile that
    # succeeds with incremental off (verified both ways, 2026-09-16 v7
    # session: same rustc invocation, SIGKILL with -C incremental, clean
    # pass with CARGO_INCREMENTAL=0). Incremental only speeds up
    # REBUILDS of the test binary; set CARGO_INCREMENTAL explicitly to
    # override.
    if [ -z "$incr" ] && [ "${dbg:-}" = "0" ]; then
        incr=0
    fi
    n="${LCCC_TEST_REPEATS:-1}"
    for ((i = 1; i <= n; i++)); do
        if [ -n "$dbg" ]; then
            export CARGO_PROFILE_FASTBUILD_DEBUG="$dbg"
        else
            unset CARGO_PROFILE_FASTBUILD_DEBUG
        fi
        if [ -n "$incr" ]; then
            export CARGO_INCREMENTAL="$incr"
        else
            unset CARGO_INCREMENTAL
        fi
        if ! RUSTFLAGS="$flags" cargo test --profile fastbuild --all-targets --locked -j "$jobs"; then
            printf 'cargo-test: FAILED on repeat %d/%d\n' "$i" "$n" >&2
            return 1
        fi
    done
}

gate "cargo-test" fast cargo_test_repeated

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

gate "overalign-typed-census" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_overalign_typed_census.sh

gate "loop-memset-decisions" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_loop_memset.sh

gate "bool-pair-tail-jmp-contract" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_bool_pair_tail_jmp.sh

gate "bool-pair-prune-atomicity" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_bool_pair_prune.sh

gate "demorgan-branch-split" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_demorgan_branch_split.sh

gate "gvn-xsign-load-cse" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_gvn_xsign_load_cse.sh

gate "segfs-declarator-codegen" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_segfs_typeof_nosteal.sh

gate "phi-acyclic-copy-order" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_phi_acyclic_order.sh

gate "doc-link-integrity" fast \
    python3 scripts/check_doc_links.py

gate "ra-web-inloop-use" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_ra_web_inloop_use.sh

gate "store-alu-cross-join" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_store_alu_cross_join.sh

gate "gla-remat-policy" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_gla_remat_policy.sh

# Per-target Speed reach-band derivation (aarch64 10, riscv64/x86-64 6,
# i686 2); self-skips legs whose cross binary/toolchain is absent.
gate "gla-cross-reach-band" fast \
    bash tests/regression/check_gla_cross_reach_band.sh

# Latch phi fed by a global address: even with every admitting knob forced
# open, zero back-edge trampolines on all four targets (structural rule).
gate "gla-backedge-no-trampoline" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_gla_backedge_no_trampoline.sh

gate "tight-loop-align" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_tight_loop_align.sh

gate "replay-gap-home-rewrite" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_replay_gap_home_rewrite.sh

gate "seg-prefix-rmw-encoding" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_seg_prefix_rmw_encoding.sh

# Full instruction×segment matrix, byte-identical to GNU as (the contract
# behind the dispatch-level segment emission; see the gate header).
gate "seg-prefix-full-matrix" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_seg_prefix_full_matrix.sh

gate "i686-atomics" fast \
    bash tests/regression/check_i686_atomics.sh

gate "i686-asm-diff" fast \
    python3 scripts/asmdiff.py --32 --lccc target/fastbuild/lccc-i686
gate "i686-tls-ie-relax" fast \
    bash tests/regression/check_i686_tls_ie_relax.sh

gate "bb-slp-redteam" fast \
    bash tests/regression/check_bb_slp_codegen.sh

# BB-SLP v3: constant-lane materialization contracts (FP gather bit
# staging, zero/ones root-splat scheduling, FP const broadcasts).
gate "bb-slp-v3" fast \
    bash tests/regression/check_bb_slp_v3_codegen.sh

# BB-SLP v4: affine window addressing, rule-(d) escape, extract
# families, review follow-ups (F1/F2/F5).
gate "bb-slp-v4" fast \
    bash tests/regression/check_bb_slp_v4_codegen.sh

# BB-SLP v5: packed shifts, rotate decomposition (both spellings,
# shared operand), Not/Neg composites, Sub(x,1) all-ones idiom, FP
# strict min/max folds, adversarial rejections.
gate "bb-slp-v5" fast \
    bash tests/regression/check_bb_slp_v5_codegen.sh

# BB-SLP v6: the 128-bit VEX memory fold, FP negation (one-instruction
# sign-mask composite), integer min/max folds, the general cmp+blendv
# composite, and the rule-(b) cross-block relaxation; plus the W5
# register-homing contracts (dead-frame-free rotate diamonds, homed
# min/max, GCC-parity shapes) and the stale-claim regression shapes.
gate "bb-slp-v6" fast \
    bash tests/regression/check_bb_slp_v6_codegen.sh

# BB-SLP v7: the red-team adversarial edges of the v6 feature set
# (cross-block corners, cmp+blendv corner predicates, FP-Neg chains,
# memfold width/order corners) plus the sub-word SELECT demotion
# (C integer promotion: i8/i16/u8/u16 selects and min/max pack at the
# lane width with predicate remapping) — tri-config differential
# (SLP on / CCC_NO_BB_SLP=1 / gcc) is part of the gate.
gate "bb-slp-v7" fast \
    bash tests/regression/check_bb_slp_v7_codegen.sh

# BB-SLP v8: struct-field stream composition (the a[i].f1/f2 unlock),
# the field-disjointness theorem (same-base/same-stride different-index
# streams), per-lane rules (c)/(d), the same-source splat (per-component
# field re-loads), Forward packs (chained seeds reusing one vector), the
# packed FMA contraction (rounding parity with the scalar gap-fused
# detector), and the scalar gap-FMA Sub extension — tri-config
# differential (SLP on / CCC_NO_BB_SLP=1 / gcc) plus the SSE2-baseline
# fail-closed run are part of the gate.
gate "bb-slp-v8" fast \
    bash tests/regression/check_bb_slp_v8_codegen.sh

# Two-block partial unroller + half-wide dword-pair SLP family: the
# guard-free xk unroll of two-block counted loops (phi threading incl.
# the IV-reference carried-phi class), the I32/U32 pair load/store/pack
# family, MemLoad stream CSE (byte-precise no-write windows), SIB
# index-var addressing, VEX in-place forms, the immediate-source load
# fold (constant-key pointer walks), and the loop-vec Max/conditional-sum
# gates — tri-config + kill-switch + SSE2-baseline differentials and the
# asm contracts (vmovq pair loads, no legacy/VEX mixing, folded cmp).
gate "two-block-unroll-redteam" fast \
    bash tests/regression/check_two_block_unroll_redteam.sh

# Adler-32 loop epic: the rolling-checksum reassociation (vpsadbw +
# vpmaddubsw weights + vpmaddwd, exact mod 2^32) that GCC/Clang/ICX all
# leave scalar — tri-config differential (epic on / -mno-avx2 / gcc) plus
# the counting-epic miscompile fixes (multi-accumulator decline, IV
# live-out materialisation, narrow-compare constant wrap) and the asm
# homing contracts.
gate "vec-adler-epic" fast \
    bash tests/regression/check_vec_adler_epic.sh

gate "cross-backend-atomics" fast \
    bash tests/regression/check_atomic_backends.sh

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

# --------------------------------------------------------------- job:bench --
# The bench workflow carries the CODEGEN-QUALITY gate, which the test job does
# not.  Leaving it out of this mirror is exactly how a +24% instruction-count
# regression on expat_xml_scan reached CI green-on-test / red-on-bench: the
# if-combine pass was predicating short-circuit chains in a UTF-8 scanner that
# can never be vectorized.  A mirror that covers only one workflow is not a
# mirror.
gate "codegen-quality-gate" fast \
    python3 .github/scripts/ci-codegen-gate.py --lccc "$LCCC"

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
