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
#   ./scripts/ci_local.sh --only NAME  # a single gate, substring match
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

gate "ci-gate-parity" fast \
    python3 scripts/check_ci_gate_parity.py

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

gate "i686-narrow-cmp-flag-law" fast \
    bash tests/regression/check_narrow_cmp_flag_law.sh

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

# The one-move phi-diamond preinitialisation hoists the cheap incoming
# above the branch. A memory-source init may fault on the path it lands
# on: `c ? *p : *q` must never touch a NULL p when c selects q. The
# hoistable-source guard (register / immediate / plain stack slot) plus
# the runtime battery and the register-init positive control live here.
gate "peephole-phi-hoist-safety" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_peephole_phi_hoist_safety.sh

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
# BOTH gates are `fast`, i.e. they run under `--fast` too.  The GitHub
# `clippy` job runs exactly this command with `-D warnings`, so a lint is a
# hard CI failure, not a slow-oracle nicety: skipping it in the pre-push
# configuration is how a red PR gets pushed.  A clippy run is ~90 s against
# the two multi-minute oracles `--fast` exists to skip.
gate "rustfmt" fast cargo fmt --all -- --check
gate "clippy" fast \
    cargo clippy --all-targets --profile fastbuild --locked -j "${CI_LOCAL_JOBS:-2}" -- -D warnings

# ------------------------------------------------------------------ done ---
hr
echo "SUMMARY: ${PASSED} passed, ${#FAILED[@]} failed, ${SKIPPED} skipped"
if [ ${#FAILED[@]} -gt 0 ]; then
    for f in "${FAILED[@]}"; do echo "  FAILED: $f"; done
    exit 1
fi
echo "ALL GATES GREEN"
