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
#   ./scripts/ci_local.sh --fast       # skip ONLY the three slow gates:
#                                      #   regression-corpus-ssa,
#                                      #   benchmark-output-oracle,
#                                      #   peephole-whitespace-invariance
#                                      # rustfmt and clippy ALWAYS run — they
#                                      # are the CI lint jobs, and skipping
#                                      # them locally is how a red PR ships.
#   ./scripts/ci_local.sh --slow       # ONLY the three slow gates (plus the
#                                      # build).  If a --fast stamp for the SAME
#                                      # tree exists, a green --slow run upgrades
#                                      # it to mode=full: fast + slow on one tree
#                                      # is every gate GitHub runs.
#   ./scripts/ci_local.sh --only NAME  # a single gate, substring match
#
# A --fast pass is NOT CI-equivalent: GitHub runs all three slow gates on every
# PR.  PR #638 went red on check_peephole_whitespace.sh after a green
# --fast run, because the snapshot gate accepted a fast stamp.  lccc-snapshot.sh
# therefore demands mode=full; obtain it with a full run or with --fast then
# --slow on the unchanged tree.
#
#   CI_LOCAL_JOBS=N   parallelism for the clippy gate (default 2).  Set 1 on
#                     low-memory hosts: `cargo clippy --all-targets` peaks
#                     near 1.6 GB resident for this crate, and OOM-killing
#                     rustc mid-gate reads as a lint failure.
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
SLOW_ONLY=0
ONLY=""
for arg in "$@"; do
    case "$arg" in
        --fast) FAST=1 ;;
        --slow) SLOW_ONLY=1 ;;
        --only) ONLY="__NEXT__" ;;
        *) if [ "$ONLY" = "__NEXT__" ]; then ONLY="$arg"; else
               echo "unknown argument: $arg" >&2; exit 2
           fi ;;
    esac
done

LCCC=target/fastbuild/lccc
# Pass stamp: the content address of the tree this run tested, written only
# when every gate is green AND the tree did not change during the run.
# lccc-snapshot.sh refuses to publish a tree without a matching stamp — a
# past revision shipped a red hosted CI because this script was skipped. Removed up front:
# a run that does not finish green leaves no proof behind.
STAMP=target/ci_local.pass
mkdir -p target
if [ "$FAST" = 1 ] && [ "$SLOW_ONLY" = 1 ]; then
    echo "--fast and --slow are complementary halves; run them one after the other" >&2
    exit 2
fi
TREE_START=$(bash scripts/worktree_tree.sh 2>/dev/null || true)
# --fast and --slow are complementary halves.  Remember the complementary
# half's stamp before it is removed: only both halves green on ONE tree may
# claim mode=full.
PRIOR_HALF_TREE=""
COMPLEMENT=""
[ "$FAST" = 1 ] && COMPLEMENT=slow
[ "$SLOW_ONLY" = 1 ] && COMPLEMENT=fast
if [ -n "$COMPLEMENT" ] && [ -z "$ONLY" ] && [ -r "$STAMP" ] &&
    [ "$(sed -n 's/^mode=//p' "$STAMP" | head -1)" = "$COMPLEMENT" ]; then
    PRIOR_HALF_TREE=$(sed -n 's/^tree=//p' "$STAMP" | head -1)
fi
rm -f "$STAMP"
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
    # --slow keeps the build: every slow gate drives the freshly built lccc.
    if [ "$SLOW_ONLY" = "1" ] && [ "$slow" != "slow" ] && [ "$name" != "build" ]; then
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

# Cheapest possible gate, and the one that catches the failure mode that is
# hardest to notice: a merge conflict committed into a shell script is a
# syntax error only when the shell REACHES the line, so every gate above the
# damage still prints PASS for a suite that never ran. Runs before `build` so
# the breakage is reported as itself rather than as a mystery later.
gate "no-conflict-markers" fast \
    python3 scripts/check_no_conflict_markers.py

# Same reasoning, and equally compiler-independent: a workflow file is not a
# shell script, so Actions step scaffolding pasted into a `run: |` block is
# valid YAML that dies with exit 127 on the hosted runner. It is invisible
# locally because no local gate parses the workflow, and it takes down every
# gate sequenced below it.
gate "ci-workflow-shell" fast \
    python3 scripts/check_ci_workflow_shell.py

# The helper scripts are tools, not gates, so the suite that runs the gates
# never runs them -- which is how `ab_interleaved.py` shipped with an import
# that cannot resolve and stayed dead for an entire series. This checks every
# helper's imports against the AST, without executing them.
gate "script-imports" fast \
    python3 scripts/check_script_imports.py

# The volatile ratchet is a pure static scan of `src/`. It needs no compiler,
# so running it before the build means a dropped `!*volatile` guard is
# reported even when the build is broken.
gate "volatile-destructuring-selftest" fast \
    python3 scripts/check_volatile_destructuring.py --self-test
gate "volatile-destructuring" fast \
    python3 scripts/check_volatile_destructuring.py

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

# CI's second test run: the fastbuild profile inherits `release`, which
# compiles every debug_assert! out -- the %rax shadow-epoch validator, the
# GVN span-lockstep check and every other debug-only invariant.  Same
# --config overrides as ci.yml ("Run tests (debug-assertions on)"), so the
# shipped fastbuild build stays assertion-free and the artifacts are cached
# under their own fingerprints.  Memory heuristics as cargo_test_repeated.
DBGASSERT_CONFIG=(--config 'profile.fastbuild.debug-assertions=true'
    --config 'profile.fastbuild.incremental=false'
    --config 'profile.fastbuild.debug=0')
cargo_test_dbgassert() {
    local flags="" jobs="${CI_LOCAL_JOBS:-2}"
    [ -r target/lccc-rustflags ] && flags="$(cat target/lccc-rustflags)"
    CARGO_INCREMENTAL=0 RUSTFLAGS="$flags" cargo test --profile fastbuild \
        --all-targets --locked -j "$jobs" "${DBGASSERT_CONFIG[@]}"
}
gate "cargo-test-debug-assertions" slow cargo_test_dbgassert

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

gate "fuzz-harness-tests" fast \
    python3 -m unittest discover -s tests/fuzz -p 'test_*.py'

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

gate "nocfi-peephole-parity" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_nocfi_peephole_parity.sh

gate "eh-frame-unwind" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_eh_frame_unwind.sh

gate "cfi-invariants" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_cfi_invariants.sh

gate "arx-frame-latency" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_arx_frame_latency.sh

gate "reassoc-latency" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_reassoc_latency.sh

gate "comdat-signature-identity" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_comdat_signature_identity.sh

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

gate "gvn-cross-block-mul-dot" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_gvn_cross_block_mul_dot.sh

gate "segfs-declarator-codegen" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_segfs_typeof_nosteal.sh

gate "tls-model-selection" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_tls_model_selection.sh

gate "call-secondary-cache" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_call_secondary_cache.sh

gate "vec-dead-remainder" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_vec_dead_remainder.sh

# ZERO-REM-2 + chained exit phis: the hoisted FMA transform carries the A
# factor in a FIXED register (%ymm1), so it may only be emitted where the
# broadcast is re-established on EVERY entry edge -- the k-unrolled chain
# shapes enter j-loop N+1 on a conditional arm of j-loop N's header.  The
# gate pins oracle parity, the one-broadcast-per-packed-loop object-code
# ratio, and the broadcast-or-refuse invariant.
gate "vec-chain-exit-phi" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_vec_chain_exit_phi.sh

# ZERO-ROT-AFFINE: the rotated latch folds `add(iv, C) < N` into `iv < N - C`
# (signed only) so the backend emits a bare-IV compare that the compare-branch
# fusion can fuse on the back edge -- no per-iteration `leaq` temporary.  The
# pass is opt-in; the gate drives it, pins the fold count, and pins the
# objected-code shape of the 4-instruction loop it produces.
gate "affine-exit-compare" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_affine_exit_compare.sh

gate "minmax-reduction" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_minmax_reduction.sh

# Observable-access gates.  Three instruments, because each catches a
# different way the `volatile` contract gets broken and each was proved to do
# so by mutation (reintroducing the guard deletion in licm.rs):
#
#   volatile-spin-loop        BEHAVIOURAL.  Asserts a volatile access is
#                             positioned between the loop header and the
#                             backward branch.  The only one of the three
#                             that catches a HOIST (a hoist keeps the access
#                             count at one, which is why the subscript gate
#                             below stays green on a miscompiled spin loop --
#                             measured, not assumed).
#   volatile-pointer-subscript BEHAVIOURAL.  Asserts the qualifier survives
#                             subscript/pointer arithmetic, i.e. no CSE, no
#                             dead-store elimination, no forward.
#   volatile-destructuring    STATIC.  Fails when a `volatile` destructured
#                             from an IR access is bound and never used, which
#                             is exactly how a guard disappears in a refactor.
#                             Costs no build, so it runs first in a review.
#
# All three are fast: they need only $CCC (or, for the static one, nothing but
# the source tree) and no oracle linkers.
gate "volatile-spin-loop" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_volatile_spin_loop.sh

gate "volatile-pointer-subscript" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_volatile_pointer_subscript.sh
gate "volatile-access-semantics" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_volatile_access_semantics.sh

# End-to-end contract for the loop-preheader pass.  Its unit tests can only
# reach the pure terminator helpers, so they cannot observe whether a
# preheader was actually inserted -- a Rust-side suite passes identically
# whether the pass fires once or never.  This gate asserts the emitted
# assembly in both directions: the shape it must improve (hoisted) and the
# shapes it must decline (byte-identical, load still in the loop).
gate "loop-preheader" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_loop_preheader.sh

# LICM must not hoist a volatile load out of its loop (C11 5.1.2.3): N
# observable accesses must not become 1.  The runtime cannot see this -- a
# hoisted volatile load computes the same answer -- so the gate asserts it
# structurally, with a non-volatile load in the same shape as the negative
# control (that one MUST be hoisted, or the test proves nothing).
# Complements the three observable-access gates above: those cover spin
# loops, subscripts and the destructuring refactor; this one covers the
# do-while shape, where a hoist is legal-looking and easy to miss.
gate "volatile-licm" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_volatile_licm.sh

# A volatile BITFIELD is a read-modify-write, sometimes straddling two storage
# units, so the gates above cannot see it: they inspect straight-line accesses.
# Every access of every one of those RMWs used to be emitted non-volatile, which
# at -O2 collapsed a loop of N volatile stores into a single store -- a wrong
# answer, not a lost optimisation.  Non-volatile twins in the same fixture are
# the control: they must still merge.
gate "volatile-bitfield" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_volatile_bitfield_split.sh

# A function DEFINITION's own `aligned(N)`: GCC honours the definition channel as
# well as the prototype channel, and the attribute REPLACES `-falign-functions`
# rather than merging with it (`aligned(1)` suppresses the directive; the
# default only applies when there is no attribute).  Both halves are asserted,
# with the parameterized spellings and two unannotated controls.
gate "function-alignment-definition" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_function_alignment_definition.sh

# One entry, one preheader.  A loop with several entries must not get a
# speculative load hoisted into a preheader that only one of its entries passes.
gate "multi-entry-loop" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_multi_entry_loop.sh

# One comparison, one set of flags. `cmov` reads EFLAGS without writing it, so
# a `cmov` chain that shares a condition needs ONE `cmp`, not one per `cmov`.
# The gate carries its own negative control (CCC_PEEPHOLE_SKIP) so a build
# where the pass silently stopped running cannot pass it.
gate "redundant-flags-compare" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_redundant_flags_compare.sh

# `subl $1, %R; testl %R, %R; jne` -- the downward counter loop -- is one
# instruction of pure overhead, because the subtraction already set ZF.  The
# arithmetic producer only agrees with `test` on ZF, so the fold is licensed by
# a consumer walk instead of by flag equality, and this gate pins both halves:
# the pair must be gone for ZF-only consumers and must SURVIVE for the
# sign-carrying `while ((n -= 3) > 0)` control in the same binary.  It also
# self-tests its own matcher and the runtime result against GCC at four
# optimisation levels -- an optimisation gate whose pattern matcher quietly
# stopped matching would otherwise pass forever.
gate "self-test-after-arith" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_self_test_after_arith.sh

# A gate must not report a FAILURE that did not happen.  Under `set -o
# pipefail` a consumer that stops reading early (`head`, `grep -q`) kills its
# producer with SIGPIPE and the pipeline then reports 141, so `if ... | grep -q`
# takes the else branch although the pattern matched -- measured at 27-153 false
# failures per 20000 checks, and reproduced end to end on a gate that failed
# about one run in three against byte-identical compiler output.  The detector
# is strict over every script CI runs; --self-test first, because a detector
# that matches nothing passes vacuously.
gate "pipefail-sigpipe-selftest" fast \
    python3 scripts/check_pipefail_sigpipe.py --self-test
gate "pipefail-sigpipe" fast \
    python3 scripts/check_pipefail_sigpipe.py

gate "hot-loop-metric" fast \
    python3 scripts/test_hot_loop_metric.py

# Pure-logic gate: it exercises the oracle-verdict / oracle-agreement
# classifier directly, so it needs neither a built linker nor a single
# installed oracle linker.  That is the point -- the paths it pins (two
# oracles going `inapplicable`, one crashing, the reference having no
# opinion) are exactly the ones a host with only bfd installed never
# executes end-to-end.
gate "linker-oracle-verdict" fast \
    python3 tests/linker/test_reloc_oracle_verdict.py

# The full corpus gate is SLOW, so a source file that cannot even link used to
# reach upstream on a green local run: tests/regression/*.c is globbed
# non-recursively and compiled standalone, so a multi-file driver dropped in
# that directory fails to link.  --compile-only is the cheap half of that gate
# (no execution, no GCC reference build) and catches exactly this, in seconds.
# Failure detail names the non-self-contained file instead of printing a wall
# of `undefined reference` lines.
gate "regression-corpus-link" fast \
    python3 tests/regression/run_regression.py --lccc "$LCCC" -j 2 --compile-only

gate "copy-alias-sizes" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_copy_alias_sizes.sh

gate "notype-code-routing" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_linker_notype_code.sh

gate "got64-old-spelling" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_got64_old_spelling.sh

gate "tls-pie-preemptible" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_tls_pie_preemptible.sh

gate "rmw-sib-folds" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_rmw_sib_folds.sh

gate "select-from-compare" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_select_from_compare.sh

gate "affine-loop-fold" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_affine_loop_fold.sh

# The FMA contract gate.  It is registered here because a regression test that
# nothing runs is the defect it was written to fix: the audit found the packed
# matmul arm contracting unconditionally, the fix shipped, and nothing pinned
# it -- so a later refactor could have removed the gate and every suite would
# still have gone green.
gate "fma-contract-gating" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_fma_gating.sh

# The matcher's legality proof: the two shapes it used to miscompile (an
# inverted-polarity break loop, an unmodeled per-iteration effect) are RUNTIME
# claims, so the gate runs them against the gcc -O0 oracle and asserts the
# exact effect counts -- plus the reach controls, so "refuse everything" does
# not pass it.
gate "fma-matcher-guards" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_fma_matcher_guards.sh

# The redundant-test elimination (a gate that existed but was never wired --
# found while auditing this round): `andl`/`orl` set exactly the flags `testl`
# would, so the test must go AND the sign flag must still be consumable from
# the logical op itself.
gate "redundant-test-elimination" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_redundant_test_elimination.sh

# The wide envelope behind that gate: 19 sizes x bound forms plus a runtime
# bound.  Two seconds, and it is the sweep that found the inclusive-bound
# miscompile -- the shell gate pins counts, this one proves the numbers.
gate "audit-envelope" fast \
    env LCCC=target/fastbuild/lccc python3 tests/regression/verify_pr713_audit.py --quiet

gate "phi-acyclic-copy-order" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_phi_acyclic_order.sh

gate "doc-link-integrity" fast \
    python3 scripts/check_doc_links.py

gate "ci-gate-parity" fast \
    python3 scripts/check_ci_gate_parity.py
gate "ci-asm-diff-parity-self-test" fast \
    python3 scripts/test_ci_gate_parity.py

gate "encdiff-semantic-validation" fast \
    python3 scripts/test_encdiff.py

# EDG/corpus compiler-free tooling contracts: the miner
# self-tests, the mocked corpus-runner verdict contracts, and the corpus
# exclusion path tests run without any compiler — they gate the TEST
# INFRASTRUCTURE itself, which generic import checks cannot.
gate "edg-changes-miner-selftest" fast \
    python3 scripts/edg_changes_mine.py selftest
gate "edg-corpus-miner-selftest" fast \
    python3 scripts/edg_corpus_mine.py selftest
gate "corpus-index-integrity" fast \
    python3 scripts/edg_corpus_mine.py verify-index
gate "edg-changes-artifact-integrity" fast \
    python3 scripts/edg_changes_mine.py verify-artifacts
gate "corpus-runner-contracts" fast \
    python3 -m unittest discover -s tests/corpus -p 'test_*.py'
gate "frontend-diagnostic-recovery" fast \
    bash tests/regression/check_frontend_diagnostic_recovery.sh
# E5 (EDG register): adversarially nested expressions used to overflow the
# 64 MB compiler thread and abort (rc=134; GCC 14 segfaults its cc1 on the
# same input). The parser now runs a frame budget and degrades to one clean
# bounded error; this gate pins crash-class rejection, deep-but-legal
# acceptance, and in-budget semantic correctness.
gate "deep-nesting-robustness" fast \
    python3 scripts/check_deep_nesting_robustness.py --lccc "$LCCC"
gate "differential-corpus-paths" fast \
    python3 scripts/test_differential_corpus_paths.py

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

gate "i686-integer-isa-parity" fast \
    bash tests/regression/check_i686_integer_isa_parity.sh

# The x86-64 64-bit map SIMD path must use two lanes (including AVX2),
# match GCC and the scalar kill switch for all tails/overlaps, and decline
# the i686 backend that cannot lower register-based Vec*I64x2 intrinsics.
gate "map-i64-two-lane" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_map_i64_two_lane.sh

gate "fortify-diagnose-as" fast \
    bash tests/regression/check_fortify_diagnose_as.sh

gate "asm-case-insensitive" fast \
    bash tests/regression/check_asm_case_insensitive.sh

gate "debug-info-flags" fast \
    bash tests/regression/check_debug_info_flags.sh

gate "dep-files" fast \
    bash tests/regression/check_dep_files.sh

gate "i686-boot-asm" fast \
    bash tests/regression/check_i686_boot_asm.sh

# ci.yml "Verify inline-asm UTF-8 assembly text".
gate "inline-asm-utf8" fast \
    python3 scripts/check_inline_asm_utf8.py --lccc "$LCCC" --expect preserved \
        --json target/inline-asm-utf8.json

# Byte-exact assembler differentials have ONE oracle: the GNU as + objdump
# 2.47 PAIR, exactly as hosted CI. Distro binutils are not interchangeable
# -- GAS 2.44 orders the i386 lea-NOP remainder after the long NOP, 2.47
# before it -- and the betterok groups accept a smaller encoding only when
# the pinned objdump proves the disassembly identical, so an unpinned
# DISASSEMBLER is an unpinned verdict authority too. Provisioned
# (idempotent, cached under ~/.cache) by the same script hosted CI runs.
gate "asm-diff-oracle-gas-2.47" fast \
    bash scripts/ensure_gas_247.sh x86_64-linux-gnu

# Whole-corpus x86-64 assembly differential (every tests/asm-diff/*.casefile,
# a one-instruction reject is never hidden by another reject): pinning a
# single follow-up file let new corpora (pc8, EVEX AVX512, XOP) land ungated.
gate "x86-asm-diff" fast \
    python3 scripts/asmdiff.py --jobs 2 --as "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as" \
        --objdump "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/objdump" \
        --lccc target/fastbuild/lccc-x86

gate "i686-asm-diff" fast \
    python3 scripts/asmdiff.py --32 --jobs 2 --as "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as" \
        --objdump "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/objdump" \
        --lccc target/fastbuild/lccc-i686
gate "i686-tls-ie-relax" fast \
    bash tests/regression/check_i686_tls_ie_relax.sh

gate "i686-narrow-cmp-flag-law" fast \
    bash tests/regression/check_narrow_cmp_flag_law.sh

# The encdiff corpus (index-fold-64: the index-fold families incl.
# segment+fold and the APX x AVX-512 EVEX oracle record; data16-branches-64:
# the 64-bit data16-branch law) is the SEMANTIC record of every deliberate
# divergence from GAS -- BEATS rows where lccc is provably shorter, DECLINED
# rows with their policy notes. Historically NO gate ran it at all; even
# once gated, the DISASSEMBLER that decides those verdicts was whatever
# objdump the host image shipped (only `as' was pinned) and the aggregate
# verdict counts were unchecked (a BEATS -> ok-best drift could hide forever). Both pins
# and the baseline are now part of the contract:
#   --as/--objdump  the 2.47 oracle PAIR from one build (the gate's bytes
#                   AND verdicts come from pinned tools),
#   --expect-histogram  the checked-in verdict-count baseline — any count
#                   change (drift, a new row, a deleted row) fails the
#                   gate until the baseline is consciously re-recorded,
#                   and its rows-sha256 digest pins row IDENTITY too (a
#                   compensating delete+add of same-verdict rows nets to
#                   zero in the counts; the digest still fails it).
# encdiff exits 1 on WRONG-BYTES/UNVERIFIED-*/REJECTS-VALID/LONGER and on a
# histogram mismatch — the exact classes this corpus exists to catch
# (ORACLE-INVALID rows -- GAS emitting the truncated data16 forms -- are
# partitioned, not failures).
gate "encdiff-corpus" fast \
    python3 scripts/encdiff.py --offline --quiet \
        --lccc target/fastbuild/lccc-x86 \
        --as "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as" \
        --objdump "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/objdump" \
        --expect-histogram tests/encdiff-corpus/expected-verdicts.txt \
        --file tests/encdiff-corpus/index-fold-64.insn \
        --file tests/encdiff-corpus/data16-branches-64.insn

# Zero-extended compare fold (fuse_zero_ext_cmp). Its OWN skip name, so an A/B
# against CCC_PEEPHOLE_SKIP=zero_ext_cmp measures this fold and not its
# sibling fuse_load_into_alu -- they used to share `load_alu_fuse`, which made
# the two inseparable. Measured worth: no runtime effect (see the gate header);
# the gate exists because BOTH failure directions are silent.
gate "zero-ext-cmp-fold" fast \
    bash tests/regression/check_zero_ext_cmp_fold.sh

# The 64-bit division width bypass (`X86Tune::bypass_div64`): the guard must be
# emitted on bypass rows only, `CCC_NO_DIV64_BYPASS=1` must restore the off-row
# text exactly, and the guarded form must compute what GCC computes.  The corpus
# fixture pins *results* on one row; this pins *which row* and *what text*, which
# is where a tuning refresh can silently go wrong.
gate "div64-bypass-asm" fast \
    bash tests/regression/check_div64_bypass_asm.sh

# Kbuild does not fingerprint compiler/linker executable contents.  Preserve
# the contract that compiler changes clean all products while linker-only
# changes retain target objects and purge only link outputs.
gate "kernel-tool-identity" fast \
    bash tests/regression/check_kernel_tool_identity.sh

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

# Nested-diamond if-conversion + sub-word SELECT demotion: the
# unique-predecessor coverage walk (dominating_deref_keys), single-entry
# single-exit arm regions, the recursive promoted-select demotion, Copy
# transparency, and the promoted-unary-lane rewrite. Enforces the
# branch-free + vectorized contracts for the whole conditional-clamp
# family, the must-branch negative controls (uncovered loads, free-barrier
# deref, side-effect/volatile arms, half-covered stores), and the
# tri-config runtime differential.
gate "bb-slp-nested-ifconv" fast \
    bash tests/regression/check_bb_slp_nested_ifconv.sh

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

# x86 SIMD ISA contract of the middle end: every vectorization entry that
# runs BEFORE the main vectorizer's gate must carry its own -mno-sse
# refusal (the const-trip map path and the memory-form ARX vectorizer
# both left XMM intrinsics in kernel TUs otherwise), -mno-avx downgrades
# to 128-bit instead of disabling, and the FMA3 fold only fires when the
# backend can emit vfmadd.  Emission AND executed semantics are pinned.
gate "vectorize-isa-gate" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_vectorize_isa_gate.sh

# Byte-compare window phase (`while (p < end && *p == *q)`, the
# match-extension shape): emission contract (32B AVX2 / 16B SSE2 windows,
# exactly one q-side page guard per phase, ctz-based exact mismatch exit),
# the kill switch, and both behavioural drivers executed against GCC under
# every width configuration.  The guard-page driver is the one that
# SIGSEGVs if the page guard is ever removed.
gate "bytecmp-window-phase" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_bytecmp_vec_codegen.sh

# Constant-array promotion: fully-constant local arrays become .rodata
# globals. Emission contracts (the .LCA_ global, store elimination,
# rip-relative references, alignment preservation) AND the fail-closed
# rejections the red-team battery drove: the variable-index runtime store
# (invisible to the original const-offset classifier — promoted straight
# into .rodata), the pointer-induction callee reader (must PROMOTE, not
# reject), and the `return a;` terminator escape (must compile without an
# ICE). Every config is executed, not just checked.
gate "const-array-promote" fast \
    bash tests/regression/check_const_array_promote.sh

# Branch-condition range fusion (range_fold): asm contracts for the fused
# sub+unsigned compare and the adversarial rejections.
gate "range-fold-branch" fast \
    bash tests/regression/check_range_fold_branch.sh

# The boot-size harnesses only run against a prepared kernel tree, so a silent
# measurement error in them is invisible to every other gate.  These two do not
# need a kernel tree: executable bytes must be summed by section FLAG (the boot
# stage keeps code in .bstext/.entrytext/.inittext, which a /^\.text/ sum
# reported as 0) and the per-object table must actually be ranked by delta
# (`sort -n` reads a %+8d column as 0 and ordered the table by object name).
gate "boot-size-measurement" fast \
    bash tests/regression/check_boot_size_measurement.sh

# Vector copy elimination.  The GP copy machinery declines every vector family
# by construction (`is_xmm_family` keeps families 24..39 out of the GP reg_refs
# bitmask, and copy_propagation / coalesce_register_copies /
# eliminate_dead_pure_writes all bail out on a register they cannot represent),
# so the copy-in/copy-out brackets the FP codegen emits around every temporary
# survived to the assembler: `__builtin_floor` as three instructions where ICX
# emits one vroundsd, `double t = a; t += b; return t;` as four where GCC, Clang
# and ICX all emit one vaddsd.  This gate pins the instruction shapes, the
# runtime equality with GCC on kernels built to hit every legality rule of the
# new pass, and a corpus ratchet so the copies cannot come back unmeasured.
gate "vector-copy-elimination" fast \
    bash tests/regression/check_vector_copy_elimination.sh

# Call-argument staging (PR #584's miscompile class): the FP liveness
# oracle's call model must never mark live SSE-argument staging dead.
# The census is legitimately dead-eliminated at non-variadic callees; the
# `# LCCC_CALL_FP` authority marker and the hardened window walk carry the
# read set past every rsp adjustment, stack-argument push and relay half.
# Runtime differential on volatile 8..12-ary f64/f32, mixed GP+SSE, SSE
# structs, negation compositions, indirect and in-loop calls, plus the
# marker/census/staging-window shape pins.
gate "call-arg-staging" fast \
    bash tests/regression/check_call_arg_staging.sh

# Loop-invariant LEA hoisting (guarded loops, pure-LEA insertion placement
# before the alignment run) + direct 3-operand RORX (no movq staging into
# the destination of a non-destructive BMI2 rotate).
gate "lea-hoist-rorx" fast \
    bash tests/regression/check_lea_hoist_rorx.sh

# Integer-parameter provenance (P1 follow-up): laundered `(T *)(uintptr_t)raw`
# must not present a Param root. The laundered copy keeps its guard while the
# genuine pointer-param copy stays guard-free (B3 intact), plus runtime
# bit-exactness vs the oracle. Negative-verified: the pre-fix tree emits the
# laundered vector loop with no guard.
gate "provenance-int-param" fast \
    bash tests/regression/check_provenance_int_param.sh

# Persist-gate C-level verdicts: the four probe nests (goto-bound init,
# runtime limit, IV-decided limit, folded limit) must keep their veto /
# allow decisions and nest shapes end to end from C sources. Negative-
# verified: a const-limit mutation of the runtime-limit probe fails exactly
# its two pins.
gate "unroll-gate-verdicts" fast \
    bash tests/regression/check_unroll_gate_verdicts.sh

# CH/MAJ oracle consensus: andn-gated mux fold, kept-earliest majority,
# acc-resident ALU consumption, zero exposed forwards (negative-verified
# against the pre-change tree: 0 andn + the loop slot shape fail there).
gate "ch-maj-codegen" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_ch_maj_codegen.sh

# Worst-15 follow-up (session 586): nbody pair-loop load CSE and same-value
# FP squares read the register once. The rbtree derived-IV recurrence was
# removed after the S46 CI RED post-mortem (scalar flavor = measured net
# loss, now opt-in via CCC_IVSR_SCALAR_DERIVED=1).
gate "nbody-perf-shapes" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_nbody_perf_shapes.sh
gate "ivsr-scalar-derived-default" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_ivsr_scalar_derived_default.sh

# linux_find_bit: the inliner's bounded-tier clone-growth budget must
# admit the three-site kernel clone (two cold self-test calls + one hot
# in-loop call) that GCC/Clang/ICX all inline, and the inlined shape
# must keep the tzcnt + andn idiom distillations.
gate "findbit-inline" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_findbit_inline.sh

# Cross-PR interaction red-team: FMA families x copy brackets x
# if-conversion x constant promotion x NonZero x the AVX1+FMA target
# class, bit-exact vs the reference compiler at matched march (canonical
# NaN patterns normalised per the C11 latitude). Found and pins two real
# defects the per-PR gates could not see (the multi-use negation peel
# and the -mno-avx2 VEX.128 ceiling).
gate "cross-pr-redteam" fast \
    bash tests/regression/check_cross_pr_redteam.sh

# FMA-negation peel: the deletion invariant.  The pass DELETES every Neg it
# absorbs, by value id and with no residual-use check, so the classification that
# feeds it must guarantee that every read of an absorbed Neg dies with the rewrite.
# The landed rule checked the wrong edge (badness flowed source -> reader while its
# own comment says reader -> source), so a Neg read by an fma argument AND by a
# materialised Neg was rewritten at the site and deleted underneath its surviving
# reader; the one-line edge flip that looks like the repair still leaves 90 of the
# 4545 enumerated shapes orphaned.  This harness re-derives the specified rule over
# that space (default <= 3 Negs), replays the pass's unit-test shapes, and asserts
# its own negative controls, so the invariant is checkable with no compiler and the
# fix can be validated before the Rust is touched.
gate "fma-peel-invariant" fast \
    python3 tools/ir_shape_check.py

# Process-global state hygiene.  Four invariants, each grep-checkable, each one
# a defect this tree actually had: tests mutated the environment behind five
# deferred-audit markers whose premise (single-threaded access) cargo's test
# pool falsifies, two modules held separate locks that serialized nothing across
# modules, and four passes read the environment per call -- eight `environ` scans
# and eight allocations per function, one of them inside a candidate loop.  The
# guard and the per-thread configs are the fix; the ratchet keeps the remaining
# 157 sites from growing while they are migrated.
gate "env-test-hygiene" fast \
    bash tests/regression/check_env_test_hygiene.sh

# The peephole used to change its mind about a line because of a trailing blank:
# a padded load stopped forwarding the constant store above it (lccc's own
# sha256 output), the %rcx address-copy fold was lost when the CONSUMER line was
# padded, and `movq $3, %xmm0` / `movq $1, %st` panicked the compiler outright --
# an index into the 16-entry GP register-name table taken before the family id
# was validated, which needs no whitespace at all.  All three are now impossible
# at the LineStore boundary; this gate re-derives the property from real
# assembly: the committed corpus, ~150k operand spellings, freshly generated
# output at four -O levels, and the assembler path, where a padded .s must
# produce byte-identical object code.
gate "peephole-whitespace-invariance" slow \
    env CCC=target/fastbuild/lccc bash tests/regression/check_peephole_whitespace.sh

# The x86 peephole driver's own comment names scripts/peephole_trace_bisect.py
# as the reliable instrument for a faulty rewrite -- and that script did not
# exist, which is how a `fold_lea_into_load` miscompile had to be bisected by
# hand while `CCC_PEEPHOLE_SKIP` named eight different "culprits" for it.  The
# tool now exists and is gated: its operand parser (the part that decides which
# dump is reported) is pinned by a self-test with a mutation proof, and the
# dynamic path is exercised on a real compile.  Sub-second, so: fast.
gate "peephole-trace-bisect" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_peephole_trace_bisect.sh
# ci.yml also runs the tool's parser self-test directly, so a broken parser
# fails even when the wrapper gate is skipped.
gate "peephole-trace-bisect-selftest" fast \
    python3 scripts/peephole_trace_bisect.py --selftest

# The one-move phi-diamond preinitialisation hoists the cheap incoming
# above the branch. A memory-source init may fault on the path it lands
# on: `c ? *p : *q` must never touch a NULL p when c selects q. The
# hoistable-source guard (register / immediate / plain stack slot) plus
# the runtime battery and the register-init positive control live here.
gate "peephole-phi-hoist-safety" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_peephole_phi_hoist_safety.sh

gate "cross-backend-atomics" fast \
    bash tests/regression/check_atomic_backends.sh

# Indexed SIB fold with a scratch-homed index (S13): the x86-64 fold decider
# must accept %rdx/%r11-homed SIB operands. Structural fold assertion on
# sqlite_put_varint plus a shrunk-corpus gcc differential — the golden-gate
# sqlite_varint/expat regression pin (mirrors the ci.yml step of the same
# script; ci-gate-parity fails if the two drift apart).
gate "indexed-fold-scratch-index" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_indexed_fold_scratch_index.sh

# _Decimal64 indexed fold: the x86-64 indexed path must accept D64 on
# the load half, the store half, and the decider. Structural SIB-movsd
# assertions plus an integer-checksum gcc differential (mirrors the ci.yml
# step of the same script; ci-gate-parity fails if the two drift apart).
gate "decimal64-indexed-fold" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_decimal64_indexed_fold.sh
gate "decimal32-arm-width" fast \
    env CCC_ARM=target/fastbuild/lccc-arm bash tests/regression/check_decimal32_indexed_fold_arm.sh

if [ -x target/fastbuild/lccc-ld ]; then
    gate "linker-fuzz" fast env \
        LCCC_LD="$PWD/target/fastbuild/lccc-ld" FUZZ_N=128 FUZZ_SEED=20260906 \
        python3 tests/linker/fuzz_ld.py
    gate "linker-elf-grammar-fuzz" fast \
        python3 tests/linker/fuzz_elf_grammar.py \
            --lccc "$PWD/target/fastbuild/lccc-ld" --iters 64 --seed 20260906
    # The whole linker suite, x86-64 + i386 (mirrors the ci.yml step;
    # ci-gate-parity requires --strict, LCCC_REQUIRE_I386=1 and the relocs
    # tool on both sides, and no --filter/--tag).  --strict fails on SKIP.
    # Fixtures assemble with the pinned GNU as 2.47 (installed above).
    # NOTE: this gate is deliberately NOT host-probe-gated.  A sandbox that
    # cannot link or execute i386 fails here, and that failure is the
    # honest report — the suite's own i386_userspace._probe() plus
    # LCCC_REQUIRE_I386=1 implement a fail-closed multilib contract, and
    # weakening it from ci_local would break the ci-gate-parity contract.
    # Per-leg host-probing belongs to the gates that lack the machinery
    # (see check_copy_alias_sizes.sh / check_linker_notype_code.sh /
    # check_nocfi_peephole_parity.sh), not to this one.
    gate "kernel-relocs-tool" fast \
        bash tests/linker/setup_kernel_tools.sh --prefix "$HOME/.cache/lccc-kernel-tools"
    gate "linker-suite" fast env \
        PATH="$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin:$PATH" \
        LCCC_REQUIRE_I386=1 \
        LCCC_RELOCS_TOOL="$HOME/.cache/lccc-kernel-tools/bin/relocs" \
        python3 tests/linker/run_linker_tests.py --lccc target/fastbuild/lccc --strict
else
    echo "SKIP  linker fuzz + linker suite (target/fastbuild/lccc-ld not built)"
    SKIPPED=$((SKIPPED + 4))
fi

# Torture-corpus provisioning contract: stamp trust, integrity manifest,
# auto-repair, invalidate-before-replace (synthetic tarball, no network).
gate "ensure-gcc-torture-contract" fast \
    bash tests/regression/check_ensure_gcc_torture.sh

# Cross-vendor oracle (docs/GODBOLT_ORACLE.md).  Only the OFFLINE half runs
# here: the live sweep needs godbolt.org, and a network dependency inside
# ci_local turns an outage into a red local gate -- the exact failure shape
# this branch already spent time on.  What is offline is the part that
# silently used to be wrong: Compiler Explorer's stream framing, and the
# program-level traps (file-scope storage, rotations by the full width) that
# make an oracle report a divergence that is really a broken test.  Both
# have shipped wrong answers before, which is why they are asserted.
gate "godbolt-oracle-selftest" fast \
    python3 tools/oracle/godbolt_oracle_selftest.py

# One CE cache now backs four tools, and every way it can be wrong is silent:
# a forgeable key serves one program's result for another's, a truncated
# record reads as a hit instead of a miss, and a cached rate-limit blip
# strands an oracle out of every future sweep. Pinned offline, no network.
gate "godbolt-cache-selftest" fast \
    python3 scripts/test_godbolt_cache.py

# MS-11: glibc `make check` triage classifier (miscompile / unsupported /
# environment). Fixture-driven, offline.
gate "glibc-triage-selftest" fast \
    python3 scripts/test_glibc_check_triage.py
gate "glibc-triage-cli-selftest" fast \
    python3 scripts/glibc_check_triage.py --self-test

# MS-01: Compiler Explorer oracle delta gate logic (metrics, tolerance,
# baseline handling). The networked run itself is hosted-only.
gate "oracle-delta-selftest" fast \
    python3 scripts/test_oracle_delta_gate.py
gate "oracle-delta-cli-selftest" fast \
    python3 scripts/oracle_delta_gate.py --self-test

# --------------------------------------------------------------- job:bench --
# The bench workflow carries the CODEGEN-QUALITY gate, which the test job does
# not.  Leaving it out of this mirror is exactly how a +24% instruction-count
# regression on expat_xml_scan reached CI green-on-test / red-on-bench: the
# if-combine pass was predicating short-circuit chains in a UTF-8 scanner that
# can never be vectorized.  A mirror that covers only one workflow is not a
# mirror.
# bench.yml "Gate self-test (stackmem accounting)": the gate's own unit tests.
gate "codegen-gate-self-test" fast \
    python3 .github/scripts/test_ci_codegen_gate.py

gate "codegen-quality-gate" fast \
    python3 .github/scripts/ci-codegen-gate.py --lccc "$LCCC"

# ------------------------------------------------------------- job:clippy --
# BOTH gates are `fast`, i.e. they run under `--fast` too.  The GitHub
# `clippy` job runs exactly this command with `-D warnings`, so a lint is a
# hard CI failure, not a slow-oracle nicety: skipping it in the pre-push
# configuration is how a red PR gets pushed.  A clippy run is ~90 s against
# the two multi-minute oracles `--fast` exists to skip.
gate "rustfmt" fast cargo fmt --all -- --check

# Memory-constrained hosts OOM-kill the clippy gate the same way they kill
# the cargo-test compile (see cargo_test_repeated): clippy-driver's metadata
# compile of the monolithic lib-test target has the same resident peak as
# rustc's, so on a < 6 GB host the OOM killer SIGKILLs it mid-gate and the
# gate reports a LINT failure that never happened (verified 2026-09-30:
# CI_LOCAL_JOBS=1 alone still SIGKILLs with the profile defaults; debug=0 +
# incremental=0 + -j1 passes in 81 s with identical lint results).  CI_LOCAL_JOBS
# stays the explicit parallelism knob; the heuristic only drops it to 1 on a
# host where the compile itself needs the headroom, mirroring cargo-test.
# Explicit CARGO_PROFILE_FASTBUILD_DEBUG / CARGO_INCREMENTAL values override
# the heuristic exactly as documented for cargo_test_repeated.
clippy_gate() {
    local flags="" dbg incr jobs total_mb
    [ -r target/lccc-rustflags ] && flags="$(cat target/lccc-rustflags)"
    jobs="${CI_LOCAL_JOBS:-2}"
    dbg="${CARGO_PROFILE_FASTBUILD_DEBUG:-}"
    incr="${CARGO_INCREMENTAL:-}"
    if [ -z "$dbg" ]; then
        total_mb=$(free -m 2>/dev/null | awk '/^Mem:/{print $2}')
        if [ -n "$total_mb" ] && [ "$total_mb" -lt 6000 ]; then
            dbg=0
            jobs=1
        fi
    fi
    if [ -z "$incr" ] && [ "${dbg:-}" = "0" ]; then
        incr=0
    fi
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
    # RUSTFLAGS reuses the build gate's resolved flags (see cargo_test_repeated):
    # without them clippy recompiles the whole crate under a different flag set.
    RUSTFLAGS="$flags" cargo clippy --all-targets --profile fastbuild \
        --locked -j "$jobs" -- -D warnings
}
gate "clippy" fast clippy_gate

# ci.yml's closing step "Regression corpus (debug-assertions compiler)": the
# corpus gates above run an assertion-free compiler, so no debug-only
# invariant (the %rax shadow-epoch validator above all) ever observes real
# codegen without this.  CI overwrites target/fastbuild/lccc with the
# assertions-ON binary because nothing after it uses the path; here later
# work does, so the binary is run from a copy and the shipping build is
# relinked from its still-cached artifacts afterwards.
regression_corpus_dbgassert() {
    local flags="" rc=0
    [ -r target/lccc-rustflags ] && flags="$(cat target/lccc-rustflags)"
    CARGO_INCREMENTAL=0 RUSTFLAGS="$flags" cargo build --profile fastbuild \
        --bin lccc --locked -j 2 "${DBGASSERT_CONFIG[@]}" || return 1
    cp -f target/fastbuild/lccc target/lccc-dbgassert || return 1
    CCC_VALIDATE_SSA=1 python3 tests/regression/run_regression.py \
        --lccc target/lccc-dbgassert -j 2 \
        --json target/regression-dbgassert-results.json || rc=1
    ./scripts/build_lccc_fast.sh >/dev/null || rc=1
    return $rc
}
gate "regression-corpus-debug-assertions" slow regression_corpus_dbgassert

# ------------------------------------------------------------------ done ---
hr
echo "SUMMARY: ${PASSED} passed, ${#FAILED[@]} failed, ${SKIPPED} skipped"
if [ ${#FAILED[@]} -gt 0 ]; then
    for f in "${FAILED[@]}"; do echo "  FAILED: $f"; done
    exit 1
fi
echo "ALL GATES GREEN"
if [ -n "$ONLY" ]; then
    echo "(--only $ONLY: partial run, no pass stamp)"
else
    TREE_END=$(bash scripts/worktree_tree.sh 2>/dev/null || true)
    if [ -n "$TREE_START" ] && [ "$TREE_START" = "$TREE_END" ]; then
        mode=full
        if [ -n "$COMPLEMENT" ]; then
            half=fast
            [ "$SLOW_ONLY" = 1 ] && half=slow
            if [ -n "$PRIOR_HALF_TREE" ] && [ "$PRIOR_HALF_TREE" = "$TREE_END" ]; then
                echo "both halves green on tree $TREE_END (--$COMPLEMENT earlier, --$half now)"
            else
                mode=$half
                echo "NOTE: stamp is mode=$half -- NOT CI-equivalent and not" \
                     "delivery-grade until --$COMPLEMENT also passes on this tree" >&2
            fi
        fi
        # The userland matters as much as the gate list: system gcc/as/ld/
        # glibc differ between hosts, and PR #638 was green on Debian 13 and
        # red on the Ubuntu runner (scripts/ci_ubuntu_chroot.sh mirrors it).
        os=$( (. /etc/os-release 2>/dev/null && echo "${ID:-unknown}-${VERSION_ID:-unknown}") ||
            echo unknown)
        printf 'tree=%s\nmode=%s\nutc=%s\nhead=%s\nos=%s\n' "$TREE_END" "$mode" \
            "$(date -u +%Y%m%dT%H%M%SZ)" "$(git rev-parse HEAD)" "$os" >"$STAMP.tmp" &&
            mv -f "$STAMP.tmp" "$STAMP"
        echo "pass stamp: $STAMP (tree $TREE_END, $mode, $os)"
    else
        echo "WARNING: the worktree changed during the run" \
             "(${TREE_START:-?} -> ${TREE_END:-?}); no pass stamp" >&2
    fi
fi
