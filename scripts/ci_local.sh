#!/usr/bin/env bash
# Run the local counterparts of the hosted compiler and tooling gates.
# Invocation parity is checked; FAST deliberately omits the slow gates below.
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
# Run `./scripts/ci_local.sh --fast` before every push; it is the local contract.
# GitHub CI runs the slow gates on every PR, so a full local run is not required.
#
#   ./scripts/ci_local.sh              # everything (test + bench + clippy)
#   ./scripts/ci_local.sh --fast       # skip ONLY the seven slow gates:
#                                      #   cargo-test-debug-assertions,
#                                      #   regression-corpus-ssa,
#                                      #   benchmark-output-oracle,
#                                      #   bounded-callgrind-corpus,
#                                      #   peephole-whitespace-invariance,
#                                      #   regression-corpus-debug-assertions,
#                                      #   switch-cross-execute (cross toolchains required)
#                                      # rustfmt and clippy ALWAYS run — they
#                                      # are the CI lint jobs, and skipping
#                                      # them locally is how a red PR ships.
#   ./scripts/ci_local.sh --slow       # ONLY the seven slow gates (plus the
#                                      # build).  If a --fast stamp for the SAME
#                                      # tree exists, a green --slow run upgrades
#                                      # it to mode=full: fast + slow on one tree
#                                      # is every gate GitHub runs.
#   ./scripts/ci_local.sh --only NAME  # a single gate, substring match
#   ./scripts/ci_local.sh --fast --skip-msrv
#       Modern-toolchain-only local validation. Omits the compatibility-floor
#       compiler, NOT any compiler correctness gate; never writes a pass stamp.
#       Default local and hosted compatibility coverage is unchanged.
#
# A --fast pass is NOT CI-equivalent: GitHub runs all seven slow gates on every
# PR.  PR #638 went red on check_peephole_whitespace.sh after a green
# --fast run, because the snapshot gate accepted a fast stamp.  lccc-snapshot.sh
# accepts a mode=fast stamp as its gate (owner decision: GitHub CI runs the
# slow gates; a local --fast pass is the snapshot requirement).
#
#   CI_LOCAL_GATE_JOBS=1|2 controls isolated compiler-free gate overlap.
#   Cargo always uses exactly two jobs; heavy gates are exclusive barriers.
#   CI_LOCAL_JOBS=N   positive explicit job count for test and Clippy gates;
#                     takes precedence over CARGO_BUILD_JOBS. Otherwise choose
#                     1 below 6 GB RAM (or if RAM cannot be detected), 2 above.
#                     Debug/incremental overrides do not disable job budgeting.
#
# Environment:
#   LCCC_TEST_REPEATS=N   run the unit-test suite N times (default 1). N > 1
#                         surfaces tests that depend on process-global state.
#
# Exit status is non-zero if ANY gate fails, and every failure is repeated in
# the summary at the end so a long log cannot hide one.
set -u -o pipefail

cd "$(dirname "$0")/.." || exit 1
# Repository root, absolute: gate commands that shell out with LCCC_REPO (or a
# script-relative fixture path) must not depend on the caller's cwd.  PR #810's
# ill-typed-builtins gate referenced $REPO without defining it (`set -u` aborted
# the runner after a green gate); defined once here for every gate.
REPO=$(pwd)
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
export PATH="$CARGO_HOME/bin:$PATH"
export CARGO_BUILD_JOBS=2
# Same fingerprints for build, normal unit tests and clippy. Debug assertions
# have their own target directory and explicitly disable incremental state.
export CARGO_PROFILE_FASTBUILD_DEBUG="${CARGO_PROFILE_FASTBUILD_DEBUG:-0}"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-1}"

FAST=0
SLOW_ONLY=0
SKIP_MSRV=0
ONLY=""
for arg in "$@"; do
    case "$arg" in
        --fast) FAST=1 ;;
        --slow) SLOW_ONLY=1 ;;
        --skip-msrv) SKIP_MSRV=1 ;;
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
# Audited isolated checks overlap; compiler/toolchain gates drain first.
# CI_LOCAL_GATE_JOBS=1 selects the serial reference (default: 2).
source scripts/ci_gate_scheduler.sh

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
    if [ "$SKIP_MSRV" = 1 ] && [ "$name" = msrv-check ]; then
        echo "SKIP  $name (--skip-msrv: modern toolchains only; no pass stamp)"
        SKIPPED=$((SKIPPED + 1))
        return 0
    fi
    # --slow keeps the build: every slow gate drives the freshly built lccc.
    if [ "$SLOW_ONLY" = "1" ] && [ "$slow" != "slow" ] && [ "$name" != "build" ]; then
        return 0
    fi
    ci_gate_submit "$name" "$@" || {
        echo "FAIL  $name (scheduler infrastructure)"
        FAILED+=("$name")
    }
}

# Cheapest possible gate, and the one that catches the failure mode that is
# hardest to notice: a merge conflict committed into a shell script is a
# syntax error only when the shell REACHES the line, so every gate above the
# damage still prints PASS for a suite that never ran. Runs before `build` so
# the breakage is reported as itself rather than as a mystery later.
gate "no-conflict-markers" fast \
    python3 scripts/check_no_conflict_markers.py

# The archive index in engineering/evidence/ARCHIVED-BULK.md is the contract
# that keeps raw bulk profiles out of the tree: PR #829's three-way merge
# resurrected 89 archived files (+749,607 lines) with zero conflicts, because
# from the PR side they were pure additions. Every archived path is a table
# row, so the check is mechanical -- and, like the conflict scan, needs no
# compiler, so it runs before `build`.
gate "archived-bulk-absent-selftest" fast \
    python3 scripts/check_archived_bulk_absent.py --self-test
gate "archived-bulk-absent" fast \
    python3 scripts/check_archived_bulk_absent.py

# Same reasoning, and equally compiler-independent: a workflow file is not a
# shell script, so Actions step scaffolding pasted into a `run: |` block is
# valid YAML that dies with exit 127 on the hosted runner. It is invisible
# locally because no local gate parses the workflow, and it takes down every
# gate sequenced below it.
# Needs PyYAML (python3-yaml on Debian/Ubuntu); this is the first YAML parser.
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

# The SIMD intrinsic surface has exactly one source of truth -- the
# generator. A hand-edited header drifting from it would silently fork the
# intrinsic contracts the frontend believes in, so byte-compare them; like
# the scans above it needs no compiler and runs before `build`. Mirrors the
# hosted Clippy-lane step (gate parity).
gate "generated-simd-header-sync" fast \
    python3 scripts/gen_lcccsimd.py --check
gate "generated-gnu-builtin-types-sync" fast \
    python3 scripts/gen_x86_builtin_types.py --check

# ---------------------------------------------------------------- build ----
gate "build" fast ./scripts/build_lccc_fast.sh

if [ ! -x "$LCCC" ]; then
    echo "FATAL: $LCCC was not produced; every later gate would be vacuous." >&2
    exit 1
fi

# The driver contract's 6,776-case -ftrapv matrix runs as one gate per ABI,
# hosted and here, so a partial run cannot silently lose an ABI.  (Its 910
# expected SIGABRTs per ABI used to cost ~2,000 s hosted: each went through
# the runner's core_pattern handler until the matrix became non-dumpable.)
gate "driver-option-contract-core" fast \
    env CCC="$LCCC" bash tests/regression/check_driver_option_contract.sh --skip-trapv-matrix

# Differential diagnostics the frontend owns: the attribute classes, the lexer
# extensions, the identifier character tables and the inline-asm
# operand/template rules are each pinned against the GCC 16.2 oracle, because
# every one of them used to be silent (an unimplemented attribute generated
# foreign-ABI code, an unmodelled constraint letter silently became a GPR).
# Hosted CI runs the same scripts.
gate "attribute-class-diagnostics" fast \
    env CCC="$LCCC" bash tests/regression/check_attribute_diagnostics.sh
gate "lexer-extensions" fast \
    env CCC="$LCCC" bash tests/regression/check_lexer_extensions.sh
gate "ucn-identifier-table" fast \
    env CCC="$LCCC" GCC="${GCC16:-gcc-16}" bash tests/regression/check_ucn_identifier_table.sh
# A character constant's TYPE decides what its text means — `u'\U0001F600'` is
# the low surrogate 0xDE00, `u8'\u00e9'` is an error, `u'ab'` keeps its last
# character — so this gate compiles and runs each case under both compilers and
# compares value, sizeof and the ordered diagnostic stream.
gate "char-literal-encoding" fast \
    env CCC="$LCCC" GCC="${GCC16:-gcc-16}" \
    python3 tests/regression/check_char_literal_encoding.py --sweep 400
gate "asm-operand-diagnostics" fast \
    env CCC="$LCCC" CCC_ARM=target/fastbuild/lccc-arm \
    CCC_RISCV=target/fastbuild/lccc-riscv CCC_I686=target/fastbuild/lccc-i686 \
    GCC="${GCC16:-gcc-16}" \
    bash tests/regression/check_asm_operand_diagnostics.sh
gate "asm-flag-outputs" fast \
    env CCC="$LCCC" CCC_I686=target/fastbuild/lccc-i686 \
    CCC_ARM=target/fastbuild/lccc-arm CCC_RISCV=target/fastbuild/lccc-riscv \
    GCC="${GCC16:-gcc-16}" \
    bash tests/regression/check_asm_flag_outputs.sh
gate "trapv-boundary-matrix-m64" fast \
    env CCC="$LCCC" bash tests/regression/check_trapv_boundary_matrix.sh --bits -m64
gate "trapv-boundary-matrix-m32" fast \
    env CCC="$LCCC" bash tests/regression/check_trapv_boundary_matrix.sh --bits -m32

# Opt-in IPA-PTA/LRS must have a local mirror as well as the dedicated hosted
# lane. Available pinned oracles are used; the dedicated lane additionally
# provisions authenticated ICX and records the sustained timing matrix.
gate "ipa-pta-live-range" fast \
    python3 tests/regression/check_ipa_pta_live_range.py \
    --lccc "$LCCC" --gcc "${GCC16:-gcc-16}" --clang "${CLANG:-clang-23}" \
    --icx "${ICX:-icx}" --reps 9 --output target/ipa-lrs-report.json

gate "default-isa-policy" fast \
    env CCC="$LCCC" bash tests/regression/check_default_isa_policy.sh
gate "sse2-widening-4lane-sum" fast \
    env CCC="$LCCC" bash tests/regression/check_sse2_widening_4lane_sum.sh
gate "cas16-lowering" fast \
    env CCC="$LCCC" bash tests/regression/check_cas16_lowering.sh
gate "target-attribute-isa" fast \
    env CCC="$LCCC" bash tests/regression/check_target_attribute_isa.sh
gate "x86-scalar-intrinsics" fast \
    env CCC="$LCCC" GCC="${GCC16:-gcc-16}" sh tests/regression/check_x86_scalar_intrinsics.sh
gate "atomic16-runtime" fast \
    env CCC="$LCCC" bash tests/regression/check_atomic16_runtime.sh
gate "i128-atomic-call-clobber" fast \
    env CCC="$LCCC" GCC16="${GCC16:-gcc-16}" \
    bash tests/regression/check_i128_atomic_call_clobber.sh
gate "i128-atomic-reproducer" fast \
    env CCC="$LCCC" GCC16="${GCC16:-gcc-16}" \
    bash tests/regression/check_i128_atomic_reproducer.sh
gate "sibcall-shapes" fast \
    env CCC="$LCCC" GCC="${GCC:-gcc-16}" \
    bash tests/regression/check_sibcall_shapes.sh
# LICM load speculation of non-faulting objects (positives hoisted, refusals
# kept, weak-undefined runtime); call-argument staging vs register homes
# (pinned __int128 stack arguments + seeds 1-6 and 11/14/16/26/30, the
# seeds that caught the staged-register scratch clobber).
gate "licm-global-speculation" fast \
    python3 tests/regression/check_licm_global_speculation.py --lccc "$LCCC" --gcc "${GCC:-gcc-16}"
gate "call-arg-staging-homes" fast \
    python3 tests/regression/check_call_arg_staging.py --lccc "$LCCC" \
    --seed-list 1,2,3,4,5,6,11,14,16,26,30
gate "fno-builtin-fold-contract" fast \
    env CCC="$LCCC" bash tests/regression/check_fno_builtin_folds.sh
gate "builtin-floor-float-narrowing" fast \
    env CCC="$LCCC" CCC_I686=target/fastbuild/lccc-i686 \
    bash tests/regression/check_builtin_floor_narrowing.sh
gate "store-forwarding-dword-reload" fast \
    env CCC="$LCCC" CCC_I686=target/fastbuild/lccc-i686 \
    bash tests/regression/check_store_forwarding_dword_reload.sh
gate "x86-execute-fixes" fast \
    env CCC="$LCCC" CCC_I686=target/fastbuild/lccc-i686 \
    bash tests/regression/check_x86_torture_execute_fixes.sh

# --------------------------------------------------------------- job:test --
# The ratchet that keeps the 2026-10-13 audit's debt numbers from growing:
# panic-class sites (AUD-ICE-1), unsafe sites, giant functions (AUD-MONO-1),
# undeclared CCC_*/LCCC_* knobs (AUD-KNOB-1) and unannotated #[ignore]s
# (AUD-ARM-GSF-1). Pure static scan -- it needs no compiler, and it must run
# before the build so a regression is reported as itself.

gate "rec2iter" fast \
    env CCC="$LCCC" bash tests/regression/check_rec2iter.sh

gate "fp-contract-parity" fast \
    env CCC="$LCCC" bash tests/regression/check_fp_contract_parity.sh

# Ten 32x32 compiles and runs: the only gate that pins the packed multiply-add
# loop on the *default* x86-64 target, i.e. the configuration where a missing
# SSE2 lowering silently costs 34% against GCC at -O2 (10.00 ms vs 6.63 ms on
# the reference kernel) without any test going red.
gate "sse2-matmul-vectorization" fast \
    env CCC="$LCCC" bash tests/regression/check_sse2_matmul_vectorization.sh
gate "sse2-i32-product" fast \
    env CCC="$LCCC" python3 tests/regression/check_sse2_i32_product.py
gate "i32-map-unroll" fast \
    env CCC="$LCCC" python3 tests/regression/check_i32_map_unroll.py
gate "fp-map-unroll" fast \
    env CCC="$LCCC" python3 tests/regression/check_fp_map_unroll.py
gate "sse1-f32-map" fast \
    env CCC="$LCCC" python3 tests/regression/check_sse1_f32_map.py
# Fixed-distance SLP: complete 256-bit F32x8/F64x4 distance expressions pack
# under fast FP semantics; strict FP, partial widths, an intervening call,
# and the no-AVX baseline all keep scalar code. The CCC_NO_FIXED_SLP leg is
# why this gate must stay wired: the kill switch once silently died when its
# per-call environment read was removed, and nothing noticed while the gate
# sat in the orphan allowlist.
gate "fixed-slp-distances" fast \
    env CCC="$LCCC" bash tests/regression/check_fixed_slp_distances_codegen.sh
# Signed dynamic limits can overflow while the FMA matmul vectorizer scales
# grouped trips into byte offsets. Exercise the I32/I64 overflow boundaries in
# both the AVX2 and two-wide paths, with packed-codegen positive controls.
gate "matmul-negative-bound" fast \
    env CCC="$LCCC" GCC_BIN="${GCC:-gcc}" \
    bash tests/regression/check_matmul_negative_bound.sh

# Two-wide matmul arm on the plain x86-64 baseline: the fused group op was its
# only 2-wide lowering, so `fp_contract == fast && -mfma` also decided whether
# the loop was vectorized -- and moving the default baseline to x86-64 v1
# silently made every matmul stencil scalar at `-O2` (N=1024: 601 ms against
# GCC 16.2's 361 ms).  Pins the packed/unfused lowering at both widths, the
# fused v3 lowering, the `-mno-sse` refusal, and that both roundings execute
# bit-exactly (the two-rounding result where no FMA3 exists).
gate "matmul-sse2-fallback" fast \
    env CCC="$LCCC" bash tests/regression/check_matmul_sse2_fallback.sh

# `_mm_cvttsd_si32` / `_mm_cvttss_si32` must TRUNCATE under every MXCSR
# rounding mode.  Both used to route through the rounding forms (`cvtsd2si` /
# `cvtss2si`), which disagree with truncation the moment the rounding mode is
# not nearest -- and, for non-integral inputs, they disagreed even at the
# default.  The gate checks the encoding (`cvtt*2si` present) and runs the
# conversion under all four rounding modes, against the GCC 16.2 oracle.
gate "sse2-truncating-convert" fast \
    env CCC="$LCCC" GCC="${GCC:-gcc-16}" bash tests/regression/check_sse2_truncating_convert.sh

# End-to-end proof that a 16-byte XMM slot store forwards into its own reload.
# The pass's unit tests prove the *rule*; this compiles real C, asserts on the
# emitted assembly (no `movdqu slot, %xmmN` may survive its `movdqu %xmmN,
# slot`), and executes every configuration against the GCC 16.2 oracle and
# against `CCC_NO_XMM_SF=1` so the rewrite is proven to be a pure optimization.
gate "sse2-slot-forwarding" fast \
    env CCC="$LCCC" GCC="${GCC:-gcc-16}" bash tests/regression/check_sse2_slot_forwarding.sh

# Every `__builtin_<libc>f` float alias GCC accepts must resolve in lccc to the
# same operation with bit-identical results (arity probed against gcc-16, then
# a runtime differential) -- the failure mode otherwise is a quiet call to a
# symbol that does not exist.
# `_mm_max_sd` / `_mm_min_sd` must be the single hardware `maxsd` / `minsd`
# with Intel's second-operand-wins tie/NaN semantics bit-exact (incl. signed
# zeros and the preserved high lane) -- they were an 84-instruction
# compare+select expansion for far too long.
gate "sse2-scalar-minmax" fast \
    env CCC="$LCCC" GCC="${GCC:-gcc-16}" bash tests/regression/check_sse2_scalar_minmax.sh

gate "builtin-float-aliases" fast \
    python3 tests/regression/check_builtin_float_aliases.py --lccc "$LCCC" --gcc "${GCC:-gcc-16}"

# The differential linker oracles (mold, lld) must install with retries and
# version pinning, never as bare apt lines whose transient mirror failure
# red-fails the whole job; this gate drives the installer against a mock
# apt and proves the failure modes are the loud ones.
gate "ci-link-oracle-provisioning" fast \
    python3 tests/regression/check_ci_link_oracle_provisioning.py

# GNU ld's default RWX LOAD diagnostic must be real on ELF32/ELF64, obey
# --fatal-warnings and last-toggle-wins, and run before flat-binary conversion.
gate "linker-rwx-segments" fast \
    python3 tests/linker/check_rwx_segments.py

# GAS `. =` / `.org` parity: macro vector layout, relaxation fixup, fill bytes,
# numeric locals and fail-closed malformed expressions on ELF32/ELF64.
gate "assembler-dot-assignment" fast \
    python3 tests/regression/check_dot_assignment.py

# Kernel preparation/build provenance: recipe identity, compiler-content
# invalidation, config preservation, real objtool and post-build re-verification.
gate "kernel-provenance" fast \
    python3 tests/regression/check_kernel_provenance.py
gate "kernel-full-harness" fast \
    python3 tests/regression/check_kernel_full_harness.py

# C11 6.2.1p4: the visible object binding types the call result; a non-callable
# shadow must not fall back to a hidden file-scope function.
gate "fnptr-result-shadowing" fast \
    bash tests/regression/check_fnptr_result_shadowing.sh

# Three compiles and two runs: pins that a loop-carried accumulator keeps its
# callee-saved home when a call inside the loop forces the value to be
# call-spanning, instead of being displaced by a loop-invariant constant that
# is only read on the loop's rare path.
gate "hot-loop-register-homes" fast \
    env CCC="$LCCC" bash tests/regression/check_hot_loop_register_homes.sh

# ~97 compiles per arm plus the runs: seconds, not minutes -- and it is the
# only gate that exercises the historical crash/miscompile corpus end to end.
gate "cpu-supports-runtime" fast \
    env CCC="$LCCC" bash tests/regression/check_cpu_supports_runtime.sh
# The __builtin_cpu_is half: name space and diagnostics, raw-field and GCC 16.2
# runtime agreement, constructor order, i686 and AArch64 legs (~1 s).
gate "cpu-is-runtime" fast \
    env CCC="$LCCC" bash tests/regression/check_cpu_is_runtime.sh

# The GOT-slot half of the same work: a local symbol referenced through its
# GOT slot must keep the symbol in the relocation (folding it produced 145
# instead of 25 with GNU ld and a fault under lccc-ld), and the x86-64 spelling
# must match the pinned GAS 2.47 oracle form by form.
gate "got-slot-local-addend" fast \
    env CCC="$LCCC" bash tests/regression/check_got_slot_local_addend.sh

gate "vex-flag-reads" fast \
    bash tests/regression/check_vex_flag_reads.sh

gate "load-cast-fold" fast \
    env CCC="$LCCC" bash tests/regression/check_load_cast_fold.sh

gate "andq-narrow-width" fast \
    env CCC="$LCCC" bash tests/regression/check_andq_narrow_width.sh

# Unit tests pin the exact dead-temporary shr/movsbl form, CFG reachability
# and skip controls. The C harness checks all 256 high bytes times six
# low-bit patterns against both unfolded code and GCC 16.
gate "shift-signext-fold" fast \
    env CCC="$LCCC" GCC=gcc-16 bash tests/regression/check_shift_signext_fold.sh

gate "generator-map" fast \
    env CCC="$LCCC" bash tests/regression/check_generator_map.sh

# RA harness self-tests: deterministic generation, GCC-only cross-opt checksum
# agreement, UBSan, and assembly-parser edge cases. The real LCCC-vs-GCC
# differential gate immediately follows after the compiler build.
gate "ra-holdout-harness" fast \
    python3 tests/ra/test_harness.py

# Differential RA/code-shape run against the compiler built by this same CI
# invocation. Checks checksums at every optimization level before recording
# per-function assembly counts; the holdout is evaluated, never tuned.
gate "ra-holdout-differential" fast \
    python3 tests/ra/spill_compare.py --suite holdout \
        --lccc "$LCCC" --gcc "${GCC16:-gcc-16}" --opt-levels 1,2,3 \
        --check --text --json target/ra-holdout.json

# Register-offset symbol folds (`Add(symbol, register)` -> `sym(%off,%idx)`):
# the sliding-window shape `const u8 *m = window + cur; m[i]` must fold the
# symbol into the memory operand where the derived pointer has no other use,
# must shrink the function when it does, must leave the RA-01 kernel dormant
# where it cannot pay (measured: identical body), must agree with GCC on
# every program's output, and must refuse PIC (no third SIB slot).
gate "reg-off-sym-fold" fast \
    env CCC="$LCCC" bash tests/regression/check_reg_off_sym_fold.sh

# A linked PIE (ET_DYN) may be loaded above 4 GiB, so no address may be
# narrowed to 32 bits. The inliner's argument bridge picked a callee's
# "parameter alloca" positionally; after a dead home was dropped the first N
# allocas were the callee's locals, and gzip's `memcpy(&v, p, sizeof v)` stored
# the POINTER into `unsigned int v` with `ty: U32` -- invisible in ET_EXEC,
# SIGSEGV under `-pie`. Rows: the kernel self-checks under `-pie` and matches
# gcc and the `-fpie` build, a struct-by-value program keeps working, and
# `CCC_VERIFY_IR=1` stays silent.
gate "pie-link-execution" fast \
    env CCC="$LCCC" bash tests/regression/check_pie_link_execution.sh

# A parameter's home is written where the callee reads it. A memory-passed
# parameter (`__int128`, `__float128`, a vector, a struct) is read out of its
# home in the entry block, before the callee's last alloca; a bridge inserted
# "after the allocas" wrote the argument after that read and a switch on an
# `__int128` argument read an unwritten slot. The store now follows the alloca
# defining its own home, an unsatisfiable site declines the inline, and a
# synthesized home owns the whole value at the parameter's own type. Rows
# compare against gcc AND against lccc with CCC_DISABLE_PASSES=inline, with a
# coverage row proving the inliner fires.
gate "inline-param-home" fast \
    env CCC="$LCCC" bash tests/regression/check_inline_param_home.sh

# Array-bound contract: a bad bound is diagnosed (never wrapped), a byte size
# that cannot exist is rejected instead of wrapping into a bogus section, and a
# huge `.bss` object is emitted sparsely under a hard RLIMIT_AS rather than
# being materialised and OOM-killed. Driven under that limit so the "constant
# memory" half is a real requirement, not a hope.
gate "array-bound-contract" fast \
    python3 scripts/check_array_bound_contract.py --lccc "$LCCC"

# UB-exploitation source tripwire (E22): scan for specific TBAA/poison
# spellings. This is not proof that the optimizer cannot exploit C undefined
# behavior: range folds can do so without an IR poison flag. Signed `-fwrapv`
# is implemented and `-fno-strict-overflow` implements its signed portion;
# the umbrella's pointer-overflow semantics remain unaudited and diagnosed.
gate "ub-exploitation-invariant" fast \
    python3 scripts/check_no_ub_exploitation.py --root src

# Sparse NOBITS `.bss` (E22): the zeros are a size, not bytes, so a 4 GiB
# array compiles, links and runs from a ~5 KB executable, a label inside a
# sparse tail keeps its offset, PROGBITS alignment follows GAS 2.47 exactly
# (no-padding `.p2align 40` accepted, unholdable padding diagnosed, never a
# signal), and negative/oversized counts follow GAS exactly.
gate "sparse-bss-nobits" fast \
    env CCC="$LCCC" bash tests/regression/check_sparse_bss_nobits.sh

# The sparse-NOBITS equivalence proof: same probes through the in-tree
# compiler must keep exit codes, stdout and .bss layout; a 16 GB object
# compiles under a hard RSS ceiling and in bounded time.
gate "sparse-bss-equivalence" fast \
    python3 tests/regression/check_sparse_bss_equivalence.py --lccc "$LCCC"

# NOBITS and `.org` parity on all four targets against pinned GAS verdicts
# (sizes and bytes; re-measure with --oracle): zero runs, pads with any fill,
# gigabyte runs, subsections and deferred skips are a size everywhere, and
# `.org` -- dropped on RISC-V, refusing the kernel ALTERNATIVE check on
# AArch64 -- follows GAS.  Deliberate divergences are listed in the script.
gate "nobits-org-parity" fast \
    python3 tests/regression/check_nobits_org_parity.py --lccc "$LCCC"

# A13/A14: a pass may synthesise a libcall only when this TU neither defines
# nor redirects that symbol (a body, an asm label, an alias or a top-level
# asm mention) nor one its libc implementation calls (memmove -> memcpy,
# puts -> fputs/fwrite/strlen), and a constant-size memcpy/memset call may be
# expanded only while the callee still is the builtin. Refusals, positive
# controls, the -fno-builtin/-ffreestanding contract, the ungated __*_chk
# rows and three runtime programs.
gate "libcall-synthesis-no-selfcall" fast \
    env CCC="$LCCC" bash tests/regression/check_libcall_synthesis_no_selfcall.sh

# Plain-`char` signedness must be one answer, not three: the type system,
# `__CHAR_UNSIGNED__`, lccc's predefined CHAR_MIN/CHAR_MAX and glibc's
# `<limits.h>` (derived from that macro) all follow `char_is_unsigned()`,
# including GCC's -fno-signed-char / -fno-unsigned-char spellings.
# `-funsigned-char` used to flip only the type, leaving `CHAR_MIN` at -128 on
# x86-64.  Behavioural rows against GCC (gcc-16 when present), plus the
# AArch64/RISC-V defaults via the binary-name target selector (preprocessing
# only).  Fast: a few tiny compiles.
gate "char-signedness" fast \
    env CCC="$LCCC" bash tests/regression/check_char_signedness.sh

# The inliner's __builtin_va_arg_pack_len() sentinel rewrite matched in the
# wrong value space, leaving a live call to the undefined
# __lccc_va_arg_pack_len whenever the cloned caller was offset (gzip 1.14's
# gnulib open-safer.c under glibc fortify failed to link).  The gate asserts
# no sentinel survives at -O0/-O1/-O2/-O3, runs the wrapper semantics
# (one/two/zero forwarded arguments) and takes gcc as the reference.
gate "va-arg-pack-len-folds" fast \
    env CCC="$LCCC" bash tests/regression/check_va_arg_pack_len_folds.sh

gate "benchmark-codegen-registry" fast \
    python3 tests/regression/check_benchmark_codegen_registry.py

gate "binary-equivalence-contracts" fast \
    python3 tests/corpus/test_binary_equivalence.py

gate "benchmark-harness-contracts" fast \
    python3 tests/benchmark/test_benchmark_harness.py
gate "benchmark-hybrid-perf-contracts" fast \
    python3 scripts/test_benchmark_hybrid_perf.py
gate "zlib-compare256-correctness" fast \
    python3 tests/regression/check_zlib_compare256.py --lccc "$LCCC" --gcc "${GCC16:-gcc-16}" --m32

gate "label-reference-cost-contracts" fast \
    python3 scripts/test_measure_label_ref_cost.py

gate "benchmark-ranking-contracts" fast \
    python3 -m unittest discover -s scripts -p test_bench_rank.py

gate "gcc-provisioning-mocks" fast \
    python3 tests/regression/check_ci_gcc16_provisioning.py

gate "swap-provisioning-mocks" fast \
    python3 tests/regression/check_ci_swap_provisioning.py

# The toolchain bundle is what replaced six 80-205 s apt transactions per
# pipeline. Its failure modes are the dangerous kind (a stale archive that
# restores anyway, a cache hit that skips a probe, an unescalated restore of
# /), so the export/restore/verify contract and the provisioning sequence are
# exercised here on every fast run -- unprivileged, against a temporary root.
gate "ci-toolchain-bundle" fast \
    python3 tests/regression/check_ci_toolchain_bundle.py

# The 703 s fast-contract step is split across two jobs; a gate listed in both
# halves is paid for twice and a gate dropped from both is silently unrun.
gate "ci-shard-partition-self-test" fast \
    python3 tests/regression/check_ci_shard_partition.py --self-test
gate "ci-shard-partition" fast \
    python3 tests/regression/check_ci_shard_partition.py

# The shard assignment itself is a reviewable record
# (.github/ci-lane-plan.json).  The workflow is what runs, so this audit is
# what keeps the record honest: a gate planned twice, planned but absent,
# present but unplanned, a shard that lost part of its byte-identical
# preamble, a drifted balance, or -- the failure that actually happened when
# the plan was keyed by step name -- a sharded job the plan does not name at
# all.  Compiler-free and under a second, so it runs with the same mock
# contracts as the partition audit above.
gate "ci-lane-plan-self-test" fast \
    python3 tests/regression/check_ci_lane_plan.py --self-test
gate "ci-lane-plan" fast \
    python3 tests/regression/check_ci_lane_plan.py

# The lane's toolchain is an INPUT CONTRACT, so it is proven before any gate
# spends minutes failing over a missing tool -- the same probe every hosted
# lane runs, with the same pin.  Run 39809624659 lost a six-minute contract job
# to "LCCC, GCC, Clang and Valgrind are all required" from the
# hexadecimal-literal oracle, because nothing had installed valgrind; the
# sibling shard lost one to a GCC 16.x that was not 16.2.  Both are two-second
# probes here, and both name the tool instead of a downstream gate.  The
# self-test is the fixture table for every failure mode the probe can report
# (missing tool, version drift, unpinned Clang), so a verifier that cannot fail
# fails this gate.
gate "oracle-toolchain-self-test" fast \
    bash scripts/ci_verify_oracles.sh --self-test
gate "gcc-oracle-installer-self-test" fast \
    bash scripts/ci_install_gcc_oracle.sh --self-test
gate "oracle-toolchain" fast \
    bash scripts/ci_verify_oracles.sh --gcc-version 16.2 --clang-version 23.1.4

gate "rust-toolchain-selector" fast \
    bash tests/regression/check_rust_toolchain_selector.sh

gate "linker-oracle-versions" fast \
    bash tests/regression/check_linker_oracle_versions.sh

gate "i686-run-policy" fast \
    bash tests/regression/check_i386_run_policy.sh

# A14-13: the type-name vector attribute must use target scalar widths and
# typedef chains, with invalid element types/counts diagnosed.
gate "ext-vector-element-size" fast \
    env CCC="$LCCC" bash tests/regression/check_ext_vector_size.sh

gate "ext-vector-padded-layout" fast \
    env CCC="$LCCC" bash tests/regression/check_ext_vector_layout.sh

# Cross-compiler by-value ABI for ext-vector returns and arguments: an lccc
# provider/caller pair and clang-compiled counterparts must agree bit for bit.
gate "ext-vector-cross-abi" fast \
    env CCC="$LCCC" bash tests/regression/check_ext_vector_abi.sh

gate "ext-vector-rvalue-subscript" fast \
    env CCC="$LCCC" bash tests/regression/check_ext_vector_rvalue_subscript.sh

# The by-value ext-vector ABI grid: every shape (lane type x lane count) on
# -m64 and -m32 at -O0/-O2, built as provider and caller by lccc and by the
# pinned Clang 23.1.4 oracle and linked in all four combinations; shapes
# Clang returns in register sequences lccc has no carrier for must be
# rejected with the by-value diagnostic. Needs the exact oracle:
# scripts/ci_install_clang_oracle.sh installs it (CLANG=... selects it).
gate "ext-vector-abi-grid" fast \
    env CCC="$LCCC" bash tests/regression/check_ext_vector_abi_grid.sh

# Vector-shaped varargs (vector_size cross-checked against GCC in all four
# link directions; ext_vector_type as pinned LCCC round-trips). A wrong fetch
# width or %al census poisons every later FP vararg, not just its own.
gate "ext-vector-varargs-abi" fast \
    env CCC="$LCCC" bash tests/regression/check_ext_vector_varargs_abi.sh

# The layout matrix is the other half of the ext-vector contract: the ABI grid
# above pins how shapes cross a call boundary, this pins what sizeof/_Alignof
# say and how lanes address inside an object, over 120 shapes x 2 targets at
# -O0 and -O2, against the pinned Clang 23.1.4 oracle.  It needs no i686
# headers (freestanding fixture) but does need the host driver to link -m32.
gate "ext-vector-layout-matrix" fast \
    env CCC="$LCCC" bash tests/regression/check_ext_vector_layout_matrix.sh
# The same probe under the AVX / AVX-512F ABI level (x86-64 only): a named
# ext vector of up to the native register width is ONE %ymm / %zmm for
# Clang, the carrier lccc shares with the vector_size ABI (AUD-VABI-2), so
# every leg must agree, returns included.
gate "ext-vector-abi-avx" fast \
    env CCC="$LCCC" bash tests/regression/check_ext_vector_abi_avx.sh

# Same strict current-tool public native-vector boundary contract as hosted
# research. This wiring does not relax either ABI or permit multilib skips.
gate "gnu-native-builtin-diagnostics" fast \
    python3 tests/regression/check_gnu_builtin_diagnostics.py --lccc "$LCCC" --gcc gcc-16

gate "public-native-simd-abi" fast \
    python3 tests/regression/check_public_simd_abi.py --lccc "$LCCC" --gcc gcc-16

# One-lane ext_vector carriers: caller-side marshalling (register class and
# width at -O0/-O1/-O2), FP/int lane-1 carrier correctness, and narrow
# add/sub/mul/neg with signedness, A/B across the small-slot configuration.
# These were orphaned (not wired into any gate) until this entry.
gate "ext-vector-callsite-abi" fast \
    env CCC="$LCCC" bash tests/regression/check_ext_vector_callsite_abi.sh
# IPA-RA must not summarize a body another TU can replace at link time
# (#pragma weak, late weak redeclaration, top-level asm .weak): a silent
# miscompile class no single-TU corpus case can reach. Six tiny links.
gate "ipa-ra-interposition" fast \
    env CCC="$LCCC" bash tests/regression/check_ipa_ra_interposition.sh
gate "ext-vector-lane1-abi" fast \
    env CCC="$LCCC" bash tests/regression/check_ext_vector_lane1_abi.sh
gate "ext-vector-narrow-arith" fast \
    env CCC="$LCCC" bash tests/regression/check_ext_vector_narrow_arith.sh

gate "a9987-salvage" fast \
    env CCC="$LCCC" bash tests/regression/check_a9987_salvage.sh

gate "memory-value-contracts" fast \
    env CCC="$LCCC" bash tests/regression/check_memory_value_contracts.sh

gate "byte-accessor-inline" fast \
    env CCC="$LCCC" bash tests/regression/check_byte_accessor_inline.sh

gate "audit-loop-contracts" fast \
    env CCC="$LCCC" bash tests/regression/check_audit_loop_contracts.sh

gate "ivsr-integer-domains" fast \
    env CCC="$LCCC" bash tests/regression/check_ivsr_domains.sh

# Block-scope static initializers (C11 6.7.9p4), designator validity (6.7.9p2-p6),
# and volatile typedef objects (6.7.8): each row is checked against GCC 16.2.
gate "static-local-init" fast \
    env CCC="$LCCC" GCC="${GCC:-gcc-16}" bash tests/regression/check_static_local_init.sh

gate "designator-init" fast \
    env CCC="$LCCC" GCC="${GCC:-gcc-16}" bash tests/regression/check_designator_init.sh

# Vector initializers inside aggregates (static, static-local, automatic;
# arrays of vectors; brace elision; same-type and integer casts), byte for
# byte against GCC 16.2 at -O0 and -O2.
gate "vector-static-init" fast \
    env CCC="$LCCC" GCC="${GCC:-gcc-16}" bash tests/regression/check_vector_static_init.sh

# GNU vector-extension whole-vector lowering: 236-function matrix run
# against GCC 16.2 at nine ISA/opt levels (bit-exact results, cross-compiler
# psABI link both ways) plus per-function instruction ceilings at
# -O2 -mavx2 (`add8` 2, GCC parity; it was 37).
gate "vector-ext-codegen" fast \
    env CCC="$LCCC" GCC="${GCC:-gcc-16}" bash tests/regression/check_vector_ext_codegen.sh

# MMX entry points: the generated 90-intrinsic __m64 differential against
# GCC's real-MMX implementation (m64 + m32) plus the packed-codegen
# witnesses (one SSE2 op on the XMM low half, no per-lane scalar loop).
gate "mmx-intrinsics" fast \
    env CCC="$LCCC" GCC="${GCC:-gcc-16}" bash tests/regression/check_mmx_intrinsics.sh

# Incoming x86-64 by-value objects, both frame modes and kill-switch control.
gate "incoming-byval-homes" fast \
    env CCC="$LCCC" GCC="${GCC:-gcc-16}" bash tests/regression/check_incoming_byval_homes.sh
gate "incoming-byval-homes-clang" fast \
    env CCC="$LCCC" ORACLE_CC="${CLANG:-clang-23}" bash tests/regression/check_incoming_byval_homes.sh

# `void` objects, members, parameters and initializers (C11 6.7.6.3p10,
# 6.7.9p3): every row's verdict and message against GCC 16.2.
gate "void-declarators" fast \
    env CCC="$LCCC" GCC="${GCC:-gcc-16}" bash tests/regression/check_void_declarators.sh

# File-scope initializers (C11 6.7.9p4) against GCC 16.2, plus runtime values of
# folded forms: a constant that is accepted but emitted as zero fails the values check.
gate "file-scope-init" fast \
    env CCC="$LCCC" GCC="${GCC:-gcc-16}" bash tests/regression/check_file_scope_init.sh

# Const-object read folding (closed PR #816, re-gated): const element/member
# reads fold to immediates; mutable statics, TLS and volatile-qualified
# accesses keep their loads. The exclusions are what kept 20 regression
# tests from miscompiling when the port first landed.
gate "const-object-fold" fast \
    env CCC="$LCCC" bash tests/regression/check_const_object_fold.sh

gate "typedef-volatile-codegen" fast \
    env CCC="$LCCC" GCC="${GCC:-gcc-16}" bash tests/regression/check_typedef_volatile_codegen.sh

# Named-space initializers through brace elision and anonymous members, against
# GCC 16.2 line by line (C11 6.7.9p17-p20, 6.7.2.2p13).
gate "seg-init-brace-elision" fast \
    env CCC="$LCCC" GCC="${GCC:-gcc-16}" bash tests/regression/check_seg_init_brace_elision.sh

# Volatile struct members (memory-mapped register blocks, nested and anonymous
# members) keep their stores and loads at -O2, against GCC 16.2 (C11 6.7.3p7).
gate "volatile-member-codegen" fast \
    env CCC="$LCCC" GCC="${GCC:-gcc-16}" bash tests/regression/check_volatile_member_codegen.sh

# A `const volatile` object is volatile at every read: the `const` qualifier
# forbids writes, it does not make the object a constant, so the read must not
# be folded away (C11 6.7.3p7).  Globals, casts, typedef-qualified objects and
# members, against GCC 16.2's per-function memory-reference counts.
gate "const-volatile-objects" fast \
    env CCC="$LCCC" GCC="${GCC:-gcc-16}" bash tests/regression/check_const_volatile_objects.sh
# nbody pair-loop family: vector-memory `(base, index, disp)` triples and
# GEP offsets through GVN Copy glue join one pointer recurrence, affine
# siblings chain (no second recurrence).  Pins the SHAPE (zero stride
# multiplies, one hoisted base materialisation) and the correctness of the
# transform at every opt level, ILP32, and under the ptr-add kill switch.

# Executed miscompile regression for the iv_widen trajectory gates and the
# exposed-latch intercept: runtime ring-top seeds over a 2^32-span marker
# buffer, gcc-parity matrix (-O0..-O3 x {default, kill switch, -m32 SKIP}).
gate "ivwiden-exposed-latch" fast \
    env CCC="$LCCC" bash tests/regression/check_ivwiden_exposed_latch.sh
# Paired-counter (sibling mirror) proof: gcc-16 runtime-edge parity with and
# without the kill switch, plus an assertion that the proof still changes the
# constant-threshold loop (a silent refusal would otherwise pass unnoticed).
gate "ivw-paired-counter" fast \
    env CCC="$LCCC" bash tests/regression/check_ivw_paired_counter.sh
gate "zlib-maketrees" fast \
    env CCC="$LCCC" bash tests/regression/check_zlib_maketrees.sh
gate "fp-extract-homes" fast \
    env CCC="$LCCC" bash tests/regression/check_fp_extract_homes.sh
gate "va-arg-pack-len" fast \
    env CCC="$LCCC" bash tests/regression/check_va_arg_pack_len.sh

# Same-compiler agreement cannot prove an ABI. Check both directions against
# GCC, including regparm/fastcall/varargs and zero-size holes, on m64 and m32.
gate "empty-aggregate-abi" fast \
    env CCC="$LCCC" bash tests/regression/check_empty_aggregate_abi.sh

gate "aggregate-call-abi" fast \
    env CCC="$LCCC" bash tests/regression/check_aggregate_call_abi.sh

# AUD-VABI-1: a 16-byte vector is ONE %xmm (SSE + SSEUP), not an INTEGER pair,
# in BOTH directions and through function pointers. Same-compiler agreement
# cannot show this -- the gate links each side against a GCC-compiled TU.
gate "vector-abi-boundary" fast \
    env CCC="$LCCC" bash tests/regression/check_vector_abi_boundary.sh

# x86-64 SysV by-value aggregates and vectors against GCC 16.2 in both call
# directions (plus lccc<->lccc): 186 curated and seeded shapes (unions,
# bit-fields, packed, nested, x87, _Float128, vectors) through scalar-only and
# aggregate-argument signatures, initializers, assignments, conditionals,
# register exhaustion and va_arg.
sysv_aggregate_abi_gate() (
    local emitted
    emitted=$(mktemp -d)
    trap 'rm -rf "$emitted"' EXIT
    python3 tests/regression/check_sysv_aggregate_abi.py --emit-only --keep "$emitted" || return
    for source in abi.h prov.c call.c; do
        [[ -s $emitted/$source ]] || {
            echo "FAIL: --emit-only did not create $source" >&2
            return 1
        }
    done
    python3 tests/regression/check_sysv_aggregate_abi.py \
        --lccc "$LCCC" --cc "${GCC:-gcc-16}"
)
gate "sysv-aggregate-abi" fast sysv_aggregate_abi_gate

# Every operand form of a vector argument (lvalues, call results, conditionals,
# comma, statement expressions, compound literals, ...) through direct,
# variadic, K&R and struct-wrapped calls at -O0 and -O2, against GCC 16.2.
gate "vector-arg-forms" fast \
    python3 tests/regression/check_vector_arg_forms.py --lccc "$LCCC" --cc "${GCC:-gcc-16}"

# AUD-VABI-2: the 32- and 64-byte case of the same rule (ONE %ymm / %zmm
# with the AVX / AVX-512F ABI level on; MEMORY without it).  Two reproducers,
# four provider/caller combinations each, -O0/-O2 x {-mavx2, x86-64-v3,
# -mavx512f (host permitting), none, -mno-avx}: every binary's output must
# equal gcc/gcc's.
gate "vector-abi-wide" fast \
    env CCC="$LCCC" GCC="${GCC:-gcc-16}" bash tests/regression/check_vector_abi_wide.sh

# ABI-EXTVEC-VA-1 and its `vector_size` twin: a vector type's alignment is its
# size (GCC `layout_type`), which fixes struct layout, object alignment and
# the >16-byte MEMORY-class argument slot; only C11 `_Alignof(type)` is capped
# at BIGGEST_ALIGNMENT. Layout constants are compared with GCC 16 verbatim
# across -m64/-mavx/-mavx512f/-m32, the x86-64 runtime cross matrix must agree
# in all four pairings, and the i386 cross legs are latched as divergent under
# ABI-GNUVEC-I386-1.
gate "gnu-vector-align-abi" fast \
    env CCC="$LCCC" bash tests/regression/check_gnu_vector_align_abi.sh

# i686-gnu-vector-abi runs after `asm-diff-oracle-gas-2.47` (below): it
# needs that gate's pinned GNU as 2.47.

# AUD-TMP-1: temp files must be symlink-proof. Drives the pre-placed-symlink
# reproducer (and its pre-placed-directory variant) against the built compiler
# and asserts the victim file is untouched, the compile still succeeds and
# nothing is written directly into $TMPDIR.
gate "temp-symlink-hardening" fast \
    env CCC="$LCCC" bash tests/regression/check_temp_symlink.sh

# AUD-DRV-1: `-o -` is stdout for -S/-E/-M (and rejected for -c / multi-input),
# never a file named `-`. Build systems pipe through this.
gate "o-stdout" fast \
    env CCC="$LCCC" bash tests/regression/check_o_stdout.sh

# AUD-HYGIENE-1: the loose `tests/*.c`/`*.sh` scratch (compat_test.sh and the
# IVSR/univsr probes) are gates now instead of unreferenced files.  The compat
# suite is a run-and-compare differential against GCC; the probe
# gate is metamorphic -- the same programs must produce the same numbers with
# the strength-reduction pass on, off, and at -O3.
gate "compat-suite" fast \
    env CCC="$LCCC" bash tests/regression/check_compat_suite.sh
# C23 acceptance is inventoried feature-by-feature against the pinned GCC
# oracle, with the current `_BitInt(17)` difference recorded explicitly.
gate "c23-compatibility-inventory" fast \
    python3 tests/regression/check_c23_inventory.py \
        --lccc "$LCCC" --gcc "${GCC16:-gcc-16}" \
        --json target/c23-inventory.json --markdown target/c23-inventory.md
# Digraph spelling is observable in macro stringification; this also exercises
# directives, token boundaries, maximal munch and native/i686 execution.
gate "c-digraphs" fast \
    python3 tests/regression/check_c_digraphs.py \
        --lccc "$LCCC" --gcc "${GCC16:-gcc-16}" --m32
# GCC warning-group parity and DWARF line/link preservation are cheap focused
# gates; keeping them in the local mirror prevents hosted-only drift.
gate "warning-compatibility-selftest" fast \
    python3 tests/regression/check_warning_compat.py --self-test
gate "warning-compatibility" fast \
    python3 tests/regression/check_warning_compat.py \
        --lccc "$LCCC" --gcc "${GCC16:-gcc-16}"
gate "warning-precedence" fast \
    python3 tests/regression/check_warning_precedence.py \
        --lccc "$LCCC" --gcc "${GCC16:-gcc-16}"
gate "macro-parity" fast \
    python3 tests/regression/check_macro_parity.py \
        --lccc "$LCCC" --gcc "${GCC16:-gcc-16}"
# Translation-phase interaction exposed by the NFS callback XDR macros.
gate "multiline-comment-splice" fast \
    env CCC="$LCCC" GCC="${GCC16:-gcc-16}" bash tests/regression/check_multiline_comment_splice.sh
gate "include-next-search" fast \
    python3 tests/regression/check_include_next_search.py \
        --lccc "$LCCC" --gcc "${GCC16:-gcc-16}"
# GP-23: the PREDEFINED macro set itself (not the warning/macro *contract* above)
# must match `gcc-16 -dM -E` for every (target, dialect) pair, with the libc
# macros that belong to the headers required ABSENT (the GP-05 ratchet, never
# allowed to grow), the essential type-size set required present, and only
# row-by-row justified value spellings tolerated.  It caught 19 real ILP32
# defects on its first run: on i686 `__INT64_C(x)` built a 32-bit `long`, the
# 80-bit x87 format was reported as 16 bytes, and the pointer-width limits
# carried the LP64 values.
gate "predefined-macro-parity" fast \
    env CCC="$LCCC" GCC="${GCC16:-gcc-16}" bash tests/regression/check_macro_parity.sh
gate "has-builtin-features" fast \
    python3 tests/regression/check_has_builtin_features.py \
        --lccc "$LCCC" --gcc "${GCC16:-gcc-16}"
gate "asm-constraints" fast \
    python3 tests/regression/check_asm_constraints.py \
        --lccc "$LCCC" --gcc "${GCC16:-gcc-16}"
gate "driver-queries" fast \
    python3 tests/regression/check_driver_queries.py \
        --lccc "$LCCC" --gcc "${GCC16:-gcc-16}"
gate "option-effect-ledger" fast \
    python3 tests/regression/check_option_effect_ledger.py \
        --lccc "$LCCC" --gcc "${GCC16:-gcc-16}"
gate "attribute-diagnostics" fast \
    env GCC="${GCC16:-gcc-16}" python3 tests/regression/check_attribute_diagnostics.py
# Kernel _calc_vm_trans selects an arm using an integer comparison. Check
# C-width-aware diagnostics, live branches and GNU ?: against GCC in both
# target modes; an untaken zero division must not become -Werror.
gate "div-zero-constant-arms" fast \
    env CCC="$LCCC" GCC="${GCC16:-gcc-16}" bash tests/regression/check_divzero_constant_arms.sh
# The kernel passes -fno-delete-null-pointer-checks unconditionally: the flag
# is an off-request (silently accepted in both modes), LCCC must never infer
# non-nullness from dereference UB (nine-shape guard ratchet at -O2/-O3), and
# the GCC differential proves the fixture still detects deletion.
gate "nullptr-check-policy" fast \
    env CCC="$LCCC" GCC="${GCC16:-gcc-16}" bash tests/regression/check_nullptr_check_policy.sh
gate "pp-directive-comment-bridge" fast \
    env CCC="$LCCC" GCC="${GCC16:-gcc-16}" bash tests/regression/check_pp_directive_comment_bridge.sh
# The kernel harness refuses to build a kernel the package does not ship; these
# are the readers that decide it (value spellings, prefix anchoring, drift
# direction, empty configs), pinned without a kernel tree.
gate "kconfig-query" fast \
    bash tests/regression/check_kconfig_query.sh
# A statement expression's value comes from the last item that can produce
# one; the kernel's `sizeof(struct { _Static_assert(...); })` idiom depends on
# it, and a used void statement expression must be diagnosed rather than
# silently initialized with whatever the block left behind.
gate "stmt-expr-tail-parity" fast \
    env CCC="$LCCC" GCC="${GCC16:-gcc-16}" CLANG="${CLANG:-clang-23}" \
        bash tests/regression/check_stmt_expr_tail_parity.sh
# Pointer-target qualifiers of implicit conversions (-Wdiscarded-qualifiers
# and the multi-level incompatible-pointer-types errors), byte-for-byte
# against required GCC 16.2, including all suppression and promotion variants.
gate "qualifier-conversions" fast \
    python3 tests/regression/check_qualifier_conversions.py \
        --lccc "$LCCC" --gcc "${GCC16:-gcc-16}"
gate "qualifier-gate-selftest" fast \
    python3 tests/regression/test_qualifier_gate.py
# -Wreturn-local-addr: GCC 16.2's front-end walk byte for byte at -O0/-O2,
# option handling both ways, and GCC's null-return code generation (native
# and i686 execution against gcc-16).
gate "return-local-addr" fast \
    python3 tests/regression/check_return_local_addr.py \
        --lccc "$LCCC" --gcc "${GCC16:-gcc-16}" --m32
gate "qualifier-composition" fast \
    python3 tests/regression/check_qualifier_composition.py \
        --lccc "$LCCC" --lccc target/fastbuild/lccc-i686 \
        --lccc target/fastbuild/lccc-arm --lccc target/fastbuild/lccc-riscv \
        --gcc "${GCC16:-gcc-16}"
gate "dwarf-line-tables" fast \
    python3 tests/regression/check_dwarf_line_tables.py \
        --lccc "$LCCC" --lccc-i686 target/fastbuild/lccc-i686 \
        --gcc "${GCC16:-gcc-16}"
# RVC: .text and relocations byte-identical to GNU as on RV64 and RV32
# (spelling-based compression, branch relaxation, `li`); SKIP without
# riscv64 binutils.
gate "riscv-rvc-parity" fast \
    python3 tests/regression/check_riscv_rvc_parity.py --lccc target/fastbuild/lccc-riscv
# -g must not change code: every historical divergence (sentinel list) on
# x86-64, i686, AArch64 and RISC-V plus a 1-in-32 sample on x86-64 and i686.
# Hosted in the unsharded backend job.
gate "debug-info-invariance" fast \
    env CCC="$LCCC" CCC_I686=target/fastbuild/lccc-i686 \
        CCC_ARM=target/fastbuild/lccc-arm CCC_RISCV=target/fastbuild/lccc-riscv \
        bash tests/regression/check_debug_info_invariance.sh
# The x86 ISA tables are generated from GCC 16.2, so they are only as true as
# the last re-measurement.  `--check` re-renders the whole table (every CPU x
# mode x negatable option, 8772 cross-checked cases) and byte-compares it with
# the committed file; the two macro gates then compare the predefined macros a
# user's `#ifdef` actually sees (204 `-march=` cases, 714 `-m`/`-mno-` cases).
# Hosted in the unsharded backend job.  `gen_x86_cpu_is_table.py --check` is
# not mirrored because it needs GCC's source tree, not just its binary.
gate "generated-x86-arch-table-sync" fast \
    env GCC="${GCC16:-gcc-16}" python3 scripts/gen_x86_arch_table.py --check
gate "x86-march-macro-parity" fast \
    python3 scripts/check_x86_march_macros.py "$LCCC" "${GCC16:-gcc-16}"
gate "x86-mflag-macro-parity" fast \
    python3 scripts/check_x86_mflag_macros.py "$LCCC" --gcc "${GCC16:-gcc-16}"
gate "ivsr-reproducers" fast \
    env CCC="$LCCC" bash tests/regression/check_ivsr_reproducers.sh


# AUD-ICE-1/AUD-DIAG-1: a malformed `__builtin_shuffle` mask must produce a
# diagnostic, not a Rust panic (and the valid forms must still lower).  The
# gate is oracle-checked against gcc, so it cannot pass by rejecting more than
# GCC does.
gate "shuffle-diagnostics" fast \
    env CCC="$LCCC" bash tests/regression/check_shuffle_diagnostics.sh

# GCC 16.2's measured diagnostic-severity contract: the C99-era
# default-error class (implicit int / implicit declaration / return
# mismatch / int conversion / incompatible pointer types), the verbatim
# read-only mutation messages, the dialect and -W flag override
# behaviour, and GCC's columns -- all oracle-checked against gcc-16 when
# present (39-entry exit table).
gate "gcc162-diagnostic-severity" fast \
    env CCC="$LCCC" GCC="${GCC16:-gcc-16}" bash tests/regression/check_gcc162_diagnostic_severity.sh

# The operand constraints of C23 6.5.2.3 and 6.5.3.2 -- `*` on a non-pointer,
# `.`/`->` on the wrong operand, the two `void` operand wordings, the
# per-context record conversion messages and the `void *` indirection warning
# with its -w/-Werror behaviour -- pinned line/column/message to gcc-16.2.
gate "operand-diagnostics" fast \
    env CCC="$LCCC" GCC="${GCC16:-gcc-16}" bash tests/regression/check_operand_diagnostics.sh

# Clang ext_vector_type component access against the Clang 23.1.4 oracle:
# LCCC's diagnostics for ext_vector_select/errors.c equal Clang's (line,
# column, message), LCCC accepts exactly the forms Clang accepts and puts the
# first error at Clang's position, and select.c prints what Clang -O0 prints
# at LCCC -O0/-O1/-O2 on -m64 and -m32.
gate "ext-vector-swizzle-diagnostics" fast \
    env CCC="$LCCC" bash tests/regression/check_ext_vector_swizzle_diagnostics.sh

# Scalar <-> GNU vector conversions: size-mismatched casts both directions,
# implicit scalar-vector assignments/initialisations at any size (GCC
# verdict for the same-size vector->scalar leg Clang accepts), or a silent
# runtime fault from the address-confusion class; oracle-checked against
# the pinned gcc/clang; the ext-vector splat and equal-size raw-reinterpret
# are running positive controls.
gate "vector-scalar-conversions" fast \
    env CCC="$LCCC" GCC="${GCC:-gcc-16}" CLANG="${CLANG:-clang-23}" \
        bash tests/regression/check_vector_scalar_conversions.sh

# C11 6.5.2.1p1: a subscript whose operands are not pointer + integer must be
# diagnosed (LCCC lowered it as a pointer access, crashing struct-based
# `__m256` code at run time); oracle-checked against gcc.
gate "subscript-diagnostics" fast \
    env CCC="$LCCC" GCC="${GCC:-gcc-16}" python3 tests/regression/check_subscript_diagnostics.py

# Sema constraints: return/assignment/call-arity rules, and the lvalue
# constraints of `=`, `op=`, `++`/`--` and `&` (an rvalue target compiled to a
# store that went nowhere); the valid lvalue forms must store and run.
gate "sema-constraints" fast \
    env CCC="$LCCC" bash tests/regression/check_sema_constraints.sh

# AUD-SILENT-ZERO-1: a target that cannot execute an x86-only intrinsic must
# trap at the first execution, not silently compute zeros (and must not trap
# anything it CAN compute).  The host cannot execute AArch64/RISC-V, so the
# gate asserts on the instruction stream AND on the trap's encoding in the
# object.
gate "cross-target-intrinsic-trap" fast \
    env CCC="$LCCC" \
    CCC_ARM=target/fastbuild/lccc-arm \
    CCC_RISCV=target/fastbuild/lccc-riscv \
    CCC_I686=target/fastbuild/lccc-i686 \
    bash tests/regression/check_cross_target_intrinsic_trap.sh

# A14-20: libc buffering must remain inside the profile-file lock.
gate "pgo-io-contract" fast \
    python3 tests/regression/check_pgo_io.py --lccc "$LCCC"

# Shared binding must preserve symbol preemption, visibility and RELA addends.
gate "riscv-shared-binding-selftest" fast \
    python3 tests/linker/test_riscv_shared_binding.py
gate "riscv-shared-binding" fast \
    python3 tests/linker/check_riscv_shared_binding.py --lccc target/fastbuild/lccc-riscv

# A14-17: shared linking must not silently consume unknown relocation kinds.
gate "riscv-relocation-contract" fast \
    python3 tests/linker/check_riscv_relocation_contract.py --lccc target/fastbuild/lccc-riscv

# Linker relaxation vs. assembler alignment/difference fixups: the same
# object linked by lccc-ld, ld.lld and GNU ld must pass its run-time checks.
gate "riscv-relax-layout" fast \
    env CCC_RISCV=target/fastbuild/lccc-riscv bash tests/regression/check_riscv_relax_layout.sh
# Relaxation against the pinned bfd (scripts/ensure_riscv_ld_247.sh); SKIPs
# without it locally, while CI passes --require.
gate "riscv-relax-bfd" fast \
    python3 tests/regression/check_riscv_relax_bfd.py --lccc target/fastbuild/lccc-riscv

gate "pr824-boundaries" fast \
    python3 tests/regression/check_pr824_boundaries.py --lccc target/fastbuild/lccc --cross --json target/redteam/boundaries.json

gate "note-run-oracle-selftest" fast \
    python3 tests/corpus/test_linker_note_runs.py

gate "pr826-harness-redteam" fast \
    python3 tests/linker/test_pr826_redteam.py

# AUD-CPU-3: an intrinsic whose instruction-set extension the invocation does
# not permit must be diagnosed (GCC/Clang reject it too), and the invocation
# that does permit it must still emit the instruction.  Oracle-checked against
# gcc wherever gcc ships the same intrinsic.
gate "intrinsic-feature-gate" fast \
    env CCC="$LCCC" bash tests/regression/check_intrinsic_feature_gate.sh

# AUD-POS-1: a diagnostic's file:line:col must survive the preprocessor -- the
# case that regressed was "every position after a #include is one line high".
# Oracle-diffed against gcc on positions, not wording.
gate "diagnostic-positions" fast \
    env CCC="$LCCC" bash tests/regression/check_diagnostic_positions.sh

# AUD-ICE-1: an ill-typed builtin call is a diagnostic, never a panic.  The
# worklist is the compiler's own name table, so a builtin added tomorrow is
# covered tomorrow; the baseline ratchets the panics and silently-accepted
# calls that exist today and fails the moment either list grows.
gate "ill-typed-builtins-selftest" fast \
    python3 scripts/check_ill_typed_builtins.py --self-test

# AUD-ICE-1: the argument counts the front end enforces for builtins come from
# include/*.h; the checked-in table is regenerated, never hand-edited, and this
# gate fails if it stops matching the headers it was derived from.
gate "sse-avx-public-api-selftest" fast \
    python3 scripts/audit_sse_avx_public_api.py --self-test
gate "sse-avx-public-api" fast \
    python3 scripts/audit_sse_avx_public_api.py --check
gate "simd-helper-contract" fast \
    python3 tests/regression/check_simd_helper_contract.py --lccc "$LCCC"
gate "native-packed-fp-helpers" fast \
    python3 tests/regression/check_legacy_sse_native.py --lccc "$LCCC" --gcc gcc-16 --source tests/regression/native_packed_fp_helpers.c --check-codegen
gate "intrinsic-arity-selftest" fast \
    python3 scripts/check_intrinsic_arity.py --self-test
gate "intrinsic-arity" fast \
    python3 scripts/check_intrinsic_arity.py --check
gate "ill-typed-builtins" fast \
    env CCC="$LCCC" LCCC_REPO=. python3 scripts/check_ill_typed_builtins.py

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
# (CARGO_PROFILE_FASTBUILD_DEBUG=0, -j 1). Debug/incremental preferences and
# explicit job overrides are independent; neither gate changes its caller's
# environment. LCCC_TEST_REPEATS accepts decimal counts 1..2147483647.
# Called only inside gate subshells. Job budgeting is independent of debug
# preference; an explicit debug=0 must not accidentally disable OOM protection.
configure_cargo_gate_resources() {
    local total_mb jobs dbg incr
    total_mb=$(free -m 2>/dev/null | awk '/^Mem:/{print $2}')
    # Bound parsing before the signed shell comparison; malformed huge probe
    # output must not bypass the conservative fallback with an integer error.
    [[ "$total_mb" =~ ^[0-9]{1,18}$ ]] || total_mb=0
    jobs="${CI_LOCAL_JOBS:-${CARGO_BUILD_JOBS:-}}"
    if [ -z "$jobs" ]; then
        if [ "$total_mb" -lt 6000 ]; then jobs=1; else jobs=2; fi
    fi
    if [[ ! "$jobs" =~ ^[1-9][0-9]*$ ]]; then
        printf 'invalid Cargo gate job count: %s\n' "$jobs" >&2
        return 2
    fi
    dbg="${CARGO_PROFILE_FASTBUILD_DEBUG:-}"
    if [ -z "$dbg" ] && [ "$total_mb" -lt 6000 ]; then dbg=0; fi
    incr="${CARGO_INCREMENTAL:-}"
    if [ -z "$incr" ] && [ "$dbg" = 0 ]; then incr=0; fi
    export CARGO_BUILD_JOBS="$jobs"
    if [ -n "$dbg" ]; then export CARGO_PROFILE_FASTBUILD_DEBUG="$dbg";
    else unset CARGO_PROFILE_FASTBUILD_DEBUG; fi
    if [ -n "$incr" ]; then export CARGO_INCREMENTAL="$incr";
    else unset CARGO_INCREMENTAL; fi
}

# A subshell preserves set/unset/empty caller environment and shell variables,
# even on an early error. No gate's automatic settings can become the next
# gate's apparent explicit overrides.
cargo_test_repeated() (
    local flags="" n i
    [ -r target/lccc-rustflags ] && flags="$(cat target/lccc-rustflags)"
    configure_cargo_gate_resources || return
    n="${LCCC_TEST_REPEATS:-1}"
    # Bound arithmetic before Bash evaluates it: huge decimals otherwise wrap
    # negative and make a requested test run silently execute zero repetitions.
    if [[ ! "$n" =~ ^[1-9][0-9]*$ || ${#n} -gt 10 ||
          ( ${#n} -eq 10 && "$n" > 2147483647 ) ]]; then
        printf 'invalid LCCC_TEST_REPEATS: %s\n' "$n" >&2
        return 2
    fi
    python3 scripts/ci_test_report.py --self-test || return
    for ((i = 1; i <= n; i++)); do
        if ! RUSTFLAGS="$flags" python3 scripts/ci_test_report.py -- \
            cargo test --profile fastbuild --all-targets --locked -j "$CARGO_BUILD_JOBS"; then
            printf 'cargo-test: FAILED on repeat %d/%d\n' "$i" "$n" >&2
            return 1
        fi
    done
)

gate "cargo-test" fast cargo_test_repeated

# The native AVX-512 masked scalar-forwarding witness is #[ignore]d because
# hosts without AVX512F cannot execute it. Run it explicitly wherever the CPU
# can; elsewhere the gate prints an explicit SKIP rather than a vacuous pass.
gate "avx512-scalar-witness-selftest" fast \
    python3 tests/regression/check_avx512_scalar_witness.py --self-test
gate "avx512-scalar-witness" fast \
    python3 tests/regression/check_avx512_scalar_witness.py

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
    local flags="" jobs=2
    [ -r target/lccc-rustflags ] && flags="$(cat target/lccc-rustflags)"
    CARGO_INCREMENTAL=0 RUSTFLAGS="$flags" python3 scripts/ci_test_report.py -- \
        cargo test --target-dir target/ci-debug --profile fastbuild \
        --all-targets --locked -j "$jobs" "${DBGASSERT_CONFIG[@]}"
}
gate "cargo-test-debug-assertions" slow cargo_test_dbgassert

# CCC_VALIDATE_SSA is what CI sets; without it the corpus does not verify SSA
# form after every pass and a malformed-IR bug can hide behind a correct
# result.
gate "regression-corpus-ssa" slow \
    env CCC_VALIDATE_SSA=1 python3 tests/regression/run_regression.py \
        --lccc "$LCCC" -j 2 --gcc gcc-16 --json regression-results.json

gate "benchmark-output-oracle-selftest" fast \
    python3 tests/corpus/test_benchmark_output_gate.py

gate "benchmark-output-oracle" slow \
    bash scripts/check_benchmark_outputs.sh

# Bounded native Callgrind surveys remain measurements, not fast smoke gates.
# Live Compiler Explorer research is not part of either CI mode.
gate "bounded-callgrind-corpus" slow \
    python3 scripts/callgrind_corpus.py --mine "$LCCC" --ref "${GCC:-gcc-16}" \
        --gcc-include-command "${GCC:-gcc-16}" --out results/callgrind-corpus

# Live Compiler Explorer surveys are explicitly invoked research tools, not CI gates.

gate "differential-correctness-oracle" fast \
    env LCCC_BIN="$LCCC" python3 tests/correctness/run_correctness.py

gate "loop-alignment-contract" fast \
    python3 scripts/check_loop_alignment.py --lccc "$LCCC"

# The loop-inversion census has a latch: a day on which the pass rotated
# nothing -- a default flipped off, its debug line
# renamed away from these regexes -- would have printed "0 rotations" and stayed
# green.  Three arms now, each with an input that reaches it: 2 when no TU yields
# a parseable record (the census itself is broken, which is a different claim from
# the optimiser being wrong), 1 when a corpus TU fails to compile (an enumeration
# that quietly shrinks is the failure mode, not the metric), and 1 when the pass is
# live but earns nothing.  6 s against the 91-TU corpus.
# Documented debt, counted rather than forgotten: a 1-3 byte struct that is a
# subobject of a larger by-value argument is mis-passed under -mregparm on i386
# (six configurations, six different wrong answers, -m64 clean). The gate latches
# the count in both directions -- growth is a regression, a drop means the debt is
# partly repaid and the budget has to be re-measured -- and refuses to call an
# empty sweep a pass.
gate "i386-small-struct-abi-debt" fast \
    bash tests/regression/check_i386_regparm_small_struct_abi.sh

# AUD-HYGIENE-1: the crash/miscompile reproducers in artifacts/repros/ are the
# compiler's scar tissue and used to be replayed by nobody. Two legs with the
# ISA pinned on BOTH compilers (an unpinned replay measures the difference
# between lccc's x86-64-v3 baseline and gcc's x86-64 default, i.e. FMA
# contraction, and reports 29 phantom failures); the declared
# not-self-contained csmith files are checked in both directions so the list
# cannot become a skip list.  See artifacts/repros/README.md.
gate "repro-replay-selftest" fast \
    python3 scripts/check_repro_replay.py --self-test
gate "repro-replay" fast \
    env CCC="$LCCC" python3 scripts/check_repro_replay.py

# Same predicate, three consumers (-Wreturn-type, switch fallthrough, statement
# expression value) and two ways to get it wrong: resolving a call's entity by
# bare name, and dropping an attribute written on a nested definition.  The faces
# are the warning count *and* the emitted control flow (lowering starts a dead block
# after a noreturn call), so rows are asserted absolutely as well as differentially
# against gcc, and `--self-test` proves the checks with canned transcripts.
gate "scoped-noreturn-diagnostics" fast \
    bash tests/regression/check_scoped_noreturn_diagnostics.sh

# `lccc -S -o - x.c` used to create a file literally named `-`, silently
# dropping the assembly from any pipeline's point of view.  GCC 14.2 pins the
# contract cell by cell (the table lives in the gate): -S/-E stream to stdout
# and create no dash file, -c fails cleanly, -MF - streams the dep rule to
# stdout with the GCC-derived object target, a derived .d is skipped when the
# output is stdout, and the full-link -o - keeps GCC's file-named-`-` parity.
# The gate is a mutation test: its --self-test rejects a fake compiler with
# the old behavior.
gate "dash-output-stdout" fast \
    bash tests/regression/check_dash_output.sh

# AUD-SILENT-ZERO-1 (gate): no x86-only builtin may compile cleanly on a
# non-x86 backend.  Main's check_x86_intrinsic_feature fires the located
# diagnostic at lowering; this sweep re-derives the x86-only name set from
# sema/builtins.rs, tries every arity 0..2 on lccc-arm and lccc-riscv, and
# asserts no clean compile and no internal-error text (mutation-tested via
# --self-test against a fake silent-zero compiler).
gate "foreign-intrinsics-rejected" fast \
    python3 scripts/check_foreign_intrinsics.py

# PR #821 F1 (2026-10-14 review): the -mgeneral-regs-only SIMD guard must
# exempt USER-authored AArch64 assembly (ordinary inline asm, naked-function
# raw bodies, top-level file-scope asm) while still rejecting every
# compiler-generated FP/SIMD reference, and the exemption must not leak past
# the #APP/#NO_APP brackets.  All rows compile real C through lccc-arm; the
# mutation self-test rejects the pre-fix compiler (which ate user SIMD) and
# a marker-less emitter.
gate "arm-inline-asm-provenance" fast \
    bash tests/regression/check_arm_inline_asm_provenance.sh

# PR #821 F3 (2026-10-14 review): in a shared library, "non-preemptible"
# does not prove "section-relative" — the null symbol (index 0), a local
# undefined symbol and a discarded-section symbol all resolve to S = 0 and
# would bake the load bias into the ADR.  The gate links hand-crafted ELF
# objects (no cross binutils needed) and pins the link-time diagnostics plus
# the valid-local control and the NONE-relocation carve-out.
gate "arm-adr-shared-contract" fast \
    bash tests/regression/check_arm_adr_shared_contract.sh

# PR #821 F2 (2026-10-14 review): input visibility and the emitted ELF must
# agree.  The gABI-merged visibility (most constraining STV_* of all
# references and the definition, order-independent) drives the export
# decision, the dynsym st_other byte and the ADR predicate — hidden
# definitions are not exported, protected ones are exported but bind
# internally, default stays preemptible.
gate "arm-visibility-shared" fast \
    bash tests/regression/check_arm_visibility_shared.sh

# i386 register-argument staging, on the emitted assembly plus the cross-compiler
# boundary: no capture may spoil an ABI source register before its last read, a
# 12-byte packed argument is never moved 16 bytes at a time, and caller/callee halves
# from *different* compilers still agree -- the one test that can see a classification
# both lccc and gcc would otherwise share a mistake in.
gate "i386-call-staging-asm" fast \
    bash tests/regression/check_i386_call_staging_asm.sh

# Pointer induction variables that advance by one constant share an index
# register, and the portable zero guards around clz/ctz are proven nonzero by
# construction.  Both are instruction-selection facts with no observable
# output, so only the emitted assembly can see them regress; the gate is
# two-sided (the pass-off build and the unguarded counts must still show the
# old shape).
gate "ptr-iv-collapse-codegen" fast \
    env CCC="$LCCC" bash tests/regression/check_ptr_iv_collapse_codegen.sh

# GCC expands a directly self-recursive callee one level; the clone removes a
# call/return, a prologue and an epilogue per level and halves the frame depth
# of a full tree walk.  The gate is two-sided (the kill-switch build must show
# the old call counts, and tail calls / oversized bodies must be untouched).
gate "self-recursion-clone" fast \
    env CCC="$LCCC" bash tests/regression/check_self_recursion_clone.sh

# The compile-time constant evaluator is depth-bounded: a constant recursion
# deeper than the bound must stay a call instead of recursing until the
# compiler's own 64 MiB stack is gone (which aborted the whole compile).
gate "ipcp-eval-depth" fast \
    env CCC="$LCCC" bash tests/regression/check_ipcp_eval_depth.sh

gate "loop-invert-census" fast \
    python3 scripts/loop_invert_census.py --lccc "$LCCC" --quiet

gate "fuzz-harness-tests" fast \
    python3 -m unittest discover -s tests/fuzz -p 'test_*.py'

gate "fuzz-engine-wiring" fast \
    python3 scripts/fuzz_diff.py --check-engines

gate "differential-fuzz-smoke" fast \
    python3 scripts/fuzz_diff.py --engine synthetic --count 4 \
        --lccc "$LCCC" --refs gcc --opts=-O0,-O2 --seed 20260906

# Object identity / value survival shapes (a local's address is not its
# contents; a conversion must not destroy its own source).
gate "addr-value-fuzz-smoke" fast \
    python3 scripts/fuzz_diff.py --engine addr_value --count 8 \
        --lccc "$LCCC" --refs gcc --opts=-O0,-O1,-O2 --seed 20261022

# Missed-alarm pairs (scripts/mutpair.py): execution-filtered known-inequivalent
# / known-equivalent near-miss pairs in one TU, plus the harness's own
# negative control (a fake optimiser must be caught) in the unit tests.
gate "mutpair-harness-tests" fast \
    sh -c 'python3 scripts/mutpair.py --self-test && python3 -m unittest scripts.test_mutpair scripts.test_aarch64_execute_suite -v'

gate "mutpair-differential-smoke" fast \
    python3 scripts/fuzz_diff.py --engine mutpair --count 4 \
        --lccc "$LCCC" --refs gcc --opts=-O0,-O2 --seed 20261019

# Minotaur-lite slice superoptimiser (scripts/slice_superopt.py): exhaustive
# 8-bit refinement proofs + wider-width gate, with missed-alarm negative
# controls.  Research tooling only -- no candidate is auto-applied to codegen.
gate "slice-superopt-tests" fast \
    sh -c 'python3 scripts/slice_superopt.py --self-test && python3 -m unittest scripts.test_slice_superopt -v'

# Real-YARPGen missed-alarm pairs (scripts/yarpgen_mutpair.py): unit tests
# run everywhere via a stub generator; the smoke gate judges real pairs
# when intel/yarpgen is provisioned (scripts/ensure_yarpgen.sh) and SKIPs
# explicitly otherwise -- an unprovisioned generator must never be a
# silent pass OR a spurious failure.
gate "yarpgen-mutpair-tests" fast \
    sh -c 'python3 scripts/yarpgen_mutpair.py --self-test && python3 -m unittest scripts.test_yarpgen_mutpair -v'

gate "yarpgen-pair-smoke" fast \
    python3 scripts/yarpgen_mutpair.py --ci-smoke --lccc "$LCCC" --refs gcc

# The provisioner's trust model as a contract: stamp/probe/drift/re-provision
# decisions, with git and cmake replaced by PATH shims (no network, no
# generator build). Mirrors the hosted step of the same name.
gate "yarpgen-provisioner-contract" fast \
    bash tests/regression/check_ensure_yarpgen.sh

gate "strict-computed-recip-codegen" fast \
    bash tests/regression/check_strict_computed_recip_codegen.sh

gate "exact-signed-division" fast \
    python3 scripts/check_exact_signed_div.py

gate "strict-packed-recip-bitwise" fast \
    python3 scripts/check_strict_packed_recip.py

# Native execution gates; static opcode controls are supplementary tripwires.
gate "fenv-local-effects" fast \
    python3 scripts/check_fenv_local_optimization.py --compiler target/fastbuild/lccc --native-only --output target/pr835-fenv --json target/pr835-fenv.json

gate "byte-count-normalization" fast \
    python3 scripts/check_byte_count_normalize.py --lccc target/fastbuild/lccc --gcc gcc-16 --output target/pr835-byte-count --json target/pr835-byte-count.json

gate "fp-environment-trap-order" fast \
    bash tests/regression/check_fp_environment.sh
# Unknown scheduler names are exclusive barriers: hashing is not a concurrent
# installer/compiler task. Network CE comparisons are deliberately NOT local CI.
gate "evidence-hash-integrity" fast python3 scripts/verify_evidence_hashes.py
gate "licm-acquire-backedge-contract" fast python3 scripts/check_licm_ordering.py

gate "ivsr-signed-affine-step" fast \
    env CCC="$LCCC" bash tests/regression/check_ivsr_signed_affine_step.sh

gate "ivsr-vecaddr-struct-walk" fast \
    env CCC="$LCCC" bash tests/regression/check_ivsr_vecaddr_struct_walk.sh

gate "strict-packed-dag" fast \
    env CCC=target/fastbuild/lccc GCC=gcc-16 bash tests/regression/check_strict_packed_dag.sh

gate "machinst-window-alloc" fast \
    bash tests/regression/check_machinst_window_alloc_wide_copy.sh

# The MachInst emitter's unresolvable-vreg fallback must replay the buffered
# IR through the default path at the original program points. The trigger
# shape ages out as the allocator improves (varargs_abi no longer trips it),
# so the gate pairs a live trigger with the varargs runtime proof.
gate "machinst-fallback-replay" fast \
    env CCC="$LCCC" bash tests/regression/check_machinst_fallback_replay.sh

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

gate "loop-store-motion" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_loop_store_motion.sh

gate "const-trip-loop-inline" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_const_trip_loop_inline.sh

gate "debug-code-parity" fast \
    env CCC=target/fastbuild/lccc python3 tests/regression/check_debug_code_parity.py

gate "fp-recurrence-and-jump-thread" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_fp_recurrence.sh

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

# Declaration-level `__seg_gs` on parameters, locals, function return types and
# typedefs (kernel srcu_read_lock_fast boot blocker): every %gs access and the
# generic-pointer contract are checked on the emitted assembly.
gate "seg-named-decl-qualifier" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_seg_named_decl_qualifier.sh

# A declaration nested inside another declaration's typeof() must not leak
# its named address space onto the outer object (kernel amd/core.c per-CPU
# read macro; oracle-compared against GCC 16).
gate "seg-typeof-stmtexpr-leak" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_seg_typeof_stmtexpr_leak.sh

# `%gs:`/`%fs:` loads whose base is a register plus a constant displacement must
# fold into the memory operand (`movq %gs:8(%rax), %rax`) — the shape GCC and
# Clang emit for every per-CPU `base->field` access — including when the base
# arrives in the accumulator from a call. The behavioural half is
# seg_gs_const_offset_fold.c (real GS base installed via arch_prctl).
gate "seg-const-offset-fold" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_seg_const_offset_fold.sh

# The result of a segment load must land in its own GPR home: `p = p->next` on
# a per-CPU pointer emits `movq %gs:24(%rdi), %rdi`, not the two-instruction
# `movq %gs:24(%rdi), %rax; movq %rax, %rdi` relay. One instruction per access
# on the kernel's hottest per-CPU shape; GCC and Clang emit the single form.
# The gate also caps the walk loop at the oracle's eight instructions.
gate "seg-direct-home" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_seg_direct_home.sh

# An explicit `-march=` is a ceiling: a TU compiled for x86-64/v2 contains no
# BMI/BMI2/AVX-class instruction, `-mno-bmi2` wins over `-mavx2`, and a
# positive control proves the check is not vacuous. The kernel decompressor's
# Makefile REPLACES KBUILD_CFLAGS (no -march on x86-64), so this is the gate
# that keeps `make`-driven builds baseline-clean: a 64-bit-startup #UD is a
# triple fault with no serial output at all — measured on QEMU, and
# indistinguishable from "the compiler miscompiled everything".
gate "isa-gate-escape" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_isa_gate_escape.sh

# The flag-only half above is necessary but not sufficient: the kernel's own
# flag-wiping Makefiles are what leaked v3 into the boot path in the first
# place.  This gate tests the patch that makes them name their ISA, refuses to
# guess when they are refactored, and the per-object artifact scan that proves
# the built boot/decompressor objects are baseline-clean.
gate "kernel-boot-isa" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_kernel_boot_isa.sh
# The front-end half of the same 6.18 kernel contract: _Generic/
# container_of_const qualifier selection (including long typeof(...)
# spellings), enum callback compatibility, bit-field promotion,
# __attribute__((cleanup)) scope guards, -Wformat parity, function-pointer
# arithmetic and string-literal address relocations in static storage.
gate "kernel-618-sema-parity" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_kernel_618_sema_parity.sh
# Executed __seg_gs runtime contract: segment accesses and aggregate copies
# run under a real GS base and match gcc -O2.
gate "seg-gs-runtime" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_seg_gs_runtime.sh

# Size sweep of __seg_gs aggregate copies: 170 sizes across the 512-byte unroll
# limit and the loop tails, each checked against gcc -O0 output at -O0 and -O2.
gate "seg-copy-size-sweep" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_seg_copy_sweep.sh

# Segment qualifier after an earlier `*` (the pointee is in %gs or %fs): the
# %fs/%gs load sequence is checked at -O0/-O1/-O2.
gate "seg-pointee-chain-asm" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_seg_pointee_chain_asm.sh

# Explicit casts between named address spaces warn as GCC 14.2 does; sizeof and
# unnamed-parameter pointer-to-segment-pointer values are checked at run time.
gate "seg-cast-warning" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_seg_cast_warning.sh

# Named-space declarator placement: each fixture's verdict must match
# its name and gcc. A parameter cannot carry a named space on its own object,
# and an array in a named space does not decay to a generic pointer.
gate "seg-declarator-space" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_seg_declarator_space.sh

# Named-space codegen through a typedef or typeof: every access to the __seg_gs
# object must carry the %gs: prefix (audit H2, silent miscompile). Mirrors the
# hosted step in ci.yml; ci-gate-parity fails if the two drift apart.
gate "seg-typedef-codegen" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_seg_typedef_codegen.sh

# Named-space conversions: a decayed array in a named space (through an
# identifier, a member of a named object, a subscript of a named array, or
# pointer arithmetic on any of these) must be rejected where gcc 14.2 rejects
# it, and accepted where it accepts.
gate "seg-conv-diagnostics" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_seg_conv_diagnostics.sh

gate "seg-target-diagnostics" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_seg_target_diagnostics.sh
# Table-driven parity: every conversion shape in seg_conv/parity_table.tsv is
# checked against GCC 16.2 and lccc. Under CI a missing gcc fails the gate (tests/regression/lib/gcc_oracle.sh).
gate "seg-conversion-parity" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_seg_conversion_parity.sh

gate "chroot-bind" fast bash tests/regression/check_chroot_bind.sh

gate "kernel-tree-clear" fast bash tests/regression/check_kernel_tree_clear.sh
# The shared mount-table helper and the chroot/kernel-tree root guards.
gate "destructive-path-guards" fast bash tests/regression/check_destructive_guards.sh

gate "tls-model-selection" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_tls_model_selection.sh

# Cross-architecture TLS model/relocation/runtime contract. This catches the
# formerly hard-coded LE lowering in PIC/shared AArch64 and RISC-V objects.
# The hosted assembler/linker lane provisions both cross toolchains. Keep
# this in the full local run, not the fast x86-only shards: compiling/linking
# the fixtures is quick, but those shards do not provision target linkers.
gate "switch-cross-execute" slow \
    python3 tests/regression/check_switch_cross_execute.py

gate "tls-cross-models" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_tls_cross_models.sh

# Hermetic oracle self-tests for the TLS runtime comparison harness
# (no compiler required). Keep exactly one registration beside its rationale.
gate "tls-runtime-oracle" fast bash tests/regression/check_tls_runtime_oracle.sh

gate "volatile-reduction" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_volatile_reduction.sh

# `__builtin_types_compatible_p` answers type IDENTITY, not type spelling
# (C11 6.2.7): Linux 6.18 asserts `__builtin_types_compatible_p(__typeof__(&f),
# void *(void *))` in arch/x86/kernel/irq.c, so a type builder that keeps
# parameter names, a typedef's spelling or a typeof's origin in the type makes
# the kernel fail to build.  Every row is compiled AND run by lccc and GCC 16.2
# and the answers are compared; `known_divergences.tsv` is checked in both
# directions so documentation cannot rot.
# An inline-asm memory operand must always carry an address.  When operand
# resolution could not compute one it printed the EMPTY string and the assembler
# got a truncated instruction (`lock  btsq  $0,`), which is how Linux 6.18
# mm/zsmalloc.c (folio_trylock -> GEN_BINARY_RMWcc) failed to build.  The gate
# asserts, at -O0..-O3 and with the kernel flag set, that no inline-asm line
# ends with a comma, that every RMWcc site shows an address, and that the
# program's output still matches GCC 16.2 -- a wrong address is as bad as a
# missing one.
gate "rmwcc-mem-operand" fast \
    env GCC="${GCC:-gcc-16}" CCC="$LCCC" python3 tests/regression/check_rmwcc_mem_operand.py

gate "types-compatible-p" fast \
    env GCC="${GCC:-gcc-16}" CCC="$LCCC" python3 tests/regression/check_types_compatible_p.py

# Named address spaces in pointer declarators and type-names
# (`pe * __seg_gs *`, `__builtin_types_compatible_p(struct s __seg_gs *,
# struct s *)`): the kernel's `__percpu` is exactly this spelling on x86.
gate "seg-pointer-declarators" fast \
    env GCC="${GCC:-gcc-16}" CCC="$LCCC" python3 tests/regression/check_seg_pointer_declarators.py

# A compound literal used as a designated array element
# (`nodemask_t node_states[3] = {[1] = ((nodemask_t){{[0]=1UL}})}` in Linux
# mm/page_alloc.c) must fold, not be diagnosed as "initializer element is not
# constant" and not be routed into the element's first member's scalar path.
gate "compound-literal-element" fast \
    env GCC="${GCC:-gcc-16}" CCC="$LCCC" python3 tests/regression/check_compound_literal_element.py

gate "asm-char-and-macro-defaults" fast \
    env CCC="$LCCC" \
    CCC_I686=target/fastbuild/lccc-i686 \
    CCC_ARM=target/fastbuild/lccc-arm \
    CCC_RISCV=target/fastbuild/lccc-riscv \
    python3 tests/regression/check_asm_char_and_macro_defaults.py

gate "gnu-retain-sections" fast \
    python3 tests/regression/check_gnu_retain_sections.py --lccc "$LCCC" --gcc "${GCC16:-gcc-16}"

# The debug knobs this series documents (`CCC_DEBUG_VOLRED`,
# `CCC_DEBUG_STATIC_INIT`, `CCC_DEBUG_TYPES_COMPATIBLE`) may explain a decision
# but must never change emitted code: a knob left exported in a benchmark or
# kernel environment would otherwise silently select an unrecorded compiler.
# Mirrors the hosted step in ci.yml; ci-gate-parity fails if the two drift apart.
gate "debug-knobs-neutral" fast \
    env CCC="$LCCC" python3 tests/regression/check_debug_knobs_neutral.py

gate "llvm-missed-inventory-tests" fast python3 scripts/test_llvm_missed_inventory.py
gate "llvm-missed-inventory-optimized-tests" fast python3 -O scripts/test_llvm_missed_inventory.py
gate "llvm-missed-zero-choice" fast python3 tests/regression/check_llvm_missed_zero_choice.py
gate "llvm-integer-observations" fast python3 tests/regression/check_llvm_integer_observations.py
gate "llvm-bit-demand" fast python3 tests/regression/check_llvm_bit_demand.py
gate "llvm-scalar-facts" fast python3 tests/regression/check_llvm_scalar_facts.py
gate "llvm-guarded-folds" fast python3 tests/regression/check_llvm_guarded_folds.py
gate "llvm-word-counts" fast python3 tests/regression/check_llvm_word_counts.py
gate "count-observations" fast python3 tests/regression/check_count_observations.py --clang "${CLANG:-clang-23}"
gate "llvm-scalar-logic" fast python3 tests/regression/check_llvm_scalar_logic.py
gate "llvm-ring-identities" fast python3 tests/regression/check_llvm_ring_identities.py
gate "llvm-predicate-relations" fast python3 tests/regression/check_llvm_predicate_relations.py
# Mask and interval folds: `(x << k) >>u k` is a mask, and {0,1} membership is one unsigned
# threshold.  Values at -O0..-O3, mechanism, a {0,2} negative control, a
# disabled-pass attribution leg and the GCC 16.2 length oracle.
gate "mask-and-interval-folds" fast python3 tests/regression/check_mask_and_interval_folds.py
# Predicate differences and ranges: `(a - b) <op> 0` reads the subtrahends,
# `(x > 3) & (x < 10)` is one unsigned compare against a wrapped difference,
# and `x < 0 ? -x : x` is the sign-mask identity.  Values at -O0..-O3, per-fold
# mechanism, five near-miss controls, disabled-pass attribution and the GCC
# 16.2 length oracle.
gate "predicate-differences-and-ranges" fast python3 tests/regression/check_predicate_differences_and_ranges.py
gate "packed-vector-relations" fast python3 tests/regression/check_packed_vector_relations.py
gate "shuffle-composition" fast python3 tests/regression/check_shuffle_composition.py
gate "packed-mask-shifts" fast python3 tests/regression/check_packed_mask_shifts.py
gate "integer-order-networks" fast python3 tests/regression/check_integer_order_networks.py --arm target/fastbuild/lccc-arm

# Dual-layout metric pair: assembly identity across serial/auto/forced, kill
# switch, host fallback and pair liveness. Mirrors the hosted step.
gate "peephole-pair-parallel" fast python3 tests/regression/check_peephole_pair_parallel.py
gate "oracle-diagnostics" fast python3 scripts/test_ci_oracle_diagnostics.py
gate "llvm-slp-oracle-builder" fast python3 scripts/test_llvm_slp_oracle_builder.py
gate "cross-linker-discovery" fast python3 tests/regression/test_check_switch_cross_execute.py

gate "tls-model-option" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_tls_model_option.sh

# Run-time witness of the dynamic TLS models (x86-64 bfd/lccc-ld/mold/lld,
# i686, and AArch64/RISC-V under qemu when the cross toolchain is installed).
gate "tls-dynamic-witness" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_tls_dynamic_witness.sh

# libgcc's unwinder through lccc-assembled frames (.cfi_* lowering on every
# target) in executables and shared libraries -- bfd, lccc-ld, mold, lld --
# which it reaches only through PT_GNU_EH_FRAME.
gate "cfi-unwind" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_cfi_unwind.sh

# Narrow-integer staging and divide width, the executed probe-boundary frame,
# and the >2 GiB CFA offsets decoded from .eh_frame bytes (`readelf` truncates
# a large CFA offset when it PRINTS it, so the gate decodes the section).
gate "narrow-int-codegen" fast \
    env CCC=target/fastbuild/lccc python3 tests/regression/check_narrow_int_codegen.py --lccc target/fastbuild/lccc --gcc gcc-16 --clang clang-23

# The codegen mode and the link mode are one decision: a position-fixed
# ET_EXEC pays no position-independent price, while -pie, -fpie, -c, -S and
# -fPIC stay position independent.
gate "default-codegen-mode" fast \
    bash tests/regression/check_default_codegen_mode.sh

# A call-free leaf pays no callee-saved register: the volatile pool carries its
# loop state, the kill switch restores the old shape, and a call the compiler
# cannot summarize still preserves its state.
gate "leaf-caller-homes" fast \
    bash tests/regression/check_leaf_caller_homes.sh

# Narrow arithmetic: wrapped values, sign extension and comparisons over the
# whole u8 grid and a sampled i16 grid, at -O0..-Os.
gate "narrow-alu-arithmetic" fast \
    bash tests/regression/check_narrow_alu_arithmetic.sh

# A symbol+index SIB consumes the index's full register, so a narrow index must
# be sign- or zero-extended before the SIB uses it. Four folds obey the law;
# the LEA arm did not, so the same `base + i` lowered two different ways.
gate "symbol-index-widths" fast \
    bash tests/regression/check_symbol_index_widths.sh

# The builtin linkers against gcc's link semantics: -shared startup files and
# default libraries, user -l before the defaults, -rpath, as-needed libm.
gate "builtin-dso-link" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_builtin_dso_link.sh

gate "f128-sign-intrinsics-strict-fp" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_f128_sign_intrinsics_strict_fp.sh

gate "fma-sqrt-intrinsics-strict-fp" fast \
    env LCCC=target/fastbuild/lccc bash tests/regression/check_fma_builtin_strict_fp.sh

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

# AFFOLD-VB: the VALUE-bounded affine exit-compare fold (`iv + C < n` ->
# `iv < n - C` behind the zero-trip guard).  Pins runtime parity with GCC at
# the LONG_MIN underflow edges, the fire count on the guarded do-while
# shapes, the refusals (non-strict, step 2, unguarded top-test), and the
# CCC_NO_AFFINE_EXIT_FOLD ownership.
gate "affine-value-bound" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_affine_value_bound.sh

gate "minmax-reduction" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_minmax_reduction.sh

# Width-aware register-resident FP reduction accumulators (in-place packed
# updates, zero stack traffic), with the surviving CCC_NO_REDUCTION_VECREG
# differential on the dot/sum-squares family.
gate "reduction-vecreg" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_reduction_vecreg_codegen.sh

# The integer twin: I32/I64 AVX2 and forced-SSE2 reduction accumulators in
# width-aware SIMD homes, spill-traffic differential on the AVX2 leg.
gate "integer-reduction-vecreg" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_integer_reduction_vecreg_codegen.sh

gate "unsigned-reduction-codegen" fast \
    bash tests/regression/check_unsigned_reduction_codegen.sh

# Integer abs comparison folds in simplify: abs(a) <= 0 -> a == 0 and
# abs(a) < k -> the biased unsigned range test, with the
# CCC_SIMPLIFY_SKIP=cmp differential ownership and the -fwrapv
# INT_MIN/LONG_MIN contract (folds suppressed; wrapping answers kept).
gate "iabs-cmp-fold" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_iabs_cmp_fold.sh

# Lowest-set-bit recognition: `1 << Ctz(y)` becomes `y & -y` with a `cmov` for
# the exact `y == 0` arm, and `0 - x` canonicalises to `Neg` (one `negl` where
# the subtraction had to materialise the zero first).  Pins the emitted shapes,
# the ownership of both kill switches and bit-exact -O0/-O2/-O3 agreement.
gate "lowest-set-bit-fold" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_lowest_set_bit_fold.sh

# The narrow negate: the C integer promotions wrap every narrow negation in a
# widen/negate/narrow round trip; `negcast` relocates the negate to the width
# the result is observed at and `zeroneg` spells `0 - x` as the `Neg` every
# consumer understands.  Pins the shapes, both kill switches, the widening and
# float controls, and -O0..-O3 agreement on every 8/16-bit input.
gate "narrow-negate-fold" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_narrow_negate_fold.sh

# Comparison dominance: two comparisons of the same value joined by `&` or `|`
# collapse to the operand that implies the other, so `(x != 0) & (x > 4)` is
# `x > 4` and `(x != 0) | (x > 4)` is `x != 0` (9 -> 4 instructions at every
# width).  Pins the shapes, both join directions, the CCC_NO_COMPARE_DOMINANCE
# ownership, the two non-dominant controls and -O0..-O3 agreement against GCC.
gate "compare-dominance-fold" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_compare_dominance_fold.sh

# Single-bit selection: `x | 1 << n` is `bts`, `x & ~(1 << n)` is `btr` and
# `(x & 1 << n) != 0` is `bt` (6/8/7 -> 3/3/4 instructions), selected by the
# new `BitSet`/`BitClear` IR operations.  Pins the shapes and the dead
# producers, the CCC_NO_BIT_SELECT ownership, the non-single-bit controls, the
# i686/AArch64/RISC-V lengths (never longer than the knob-off build) and
# -O0..-O3 agreement against GCC.
gate "bit-select-fold" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_bit_select_fold.sh

# `c ? 1 : 0` is `c != 0` and `c ? 0 : 1` is `c == 0` (5 -> 4), and
# `(a != 0) | (b != 0)` is `(a | b) != 0` (9 -> 4).  Pins the folded shapes
# and their missing `cmov`, the refusals, the CCC_NO_PREDICATE_MATERIALIZE
# ownership, the i686/AArch64/RISC-V lengths, -O0..-O3 agreement against GCC,
# and the vectorizer's mask store (a stored comparison is 0/1, a stored lane
# mask is 0/-1).
gate "predicate-materialize" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_predicate_materialize.sh

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

# A sign-only conditional after arithmetic can consume SUB's sign flag directly:
# TEST clears OF, making TEST's `ge`/`l` exactly SUB's `ns`/`s`. The gate is
# wired DIRECTLY in both mirrors (not through a wrapper's option), checks
# signed-wrap edge outputs against GCC/Clang/ICX, exercises the dedicated
# `sign_test_cmov` skip group, and pins strict `> 0` as a negative control.
# The i686 leg uses the same fixture under -m32 with its own CFG proof.
gate "arithmetic-test-sign-cmov" fast \
    env CCC=target/fastbuild/lccc \
    GCC_SIGN_CMOV="${GCC16:-gcc-16}" CLANG_SIGN_CMOV="${CLANG:-clang}" \
    ICX_SIGN_CMOV="${ICX:-}" LCCC_FASTBUILD_BIN=target/fastbuild/lccc \
    bash tests/regression/check_sign_test_cmov.sh

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

# The inverse trap: errexit ignores a pipeline inverted with `!`, so a bare
# `! grep -q FORBIDDEN out.s` never fails its gate.  43 such assertions were
# dead when this landed, two of them over wrong premises.
gate "errexit-negation-selftest" fast \
    python3 scripts/check_errexit_negation.py --self-test
gate "errexit-negation" fast \
    python3 scripts/check_errexit_negation.py

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
gate "linker-fixture-assembler" fast \
    python3 tests/linker/test_fixture_assembler.py

# The full corpus gate is SLOW, so a source file that cannot even link used to
# reach upstream on a green local run: tests/regression/*.c is globbed
# non-recursively and compiled standalone, so a multi-file driver dropped in
# that directory fails to link.  --compile-only is the cheap half of that gate
# (no execution, no GCC reference build) and catches exactly this, in seconds.
# Failure detail names the non-self-contained file instead of printing a wall
# of `undefined reference` lines.
gate "regression-corpus-link" fast \
    python3 tests/regression/run_regression.py --lccc "$LCCC" -j 2 --compile-only

# A nested same-tag record definition must not alias the outer one. The
# regression corpus cannot see this class: it compares runtime output against
# GCC, and GCC refuses to build the offending program at all, so the oracle
# here is accept/reject. TAG-ID-1 -- it shipped as a miscompile (44 where the
# answer is 300) with every corpus and unit test green.
gate "record-tag-identity" fast \
    env CCC="$LCCC" bash tests/regression/check_record_tag_identity.sh
gate "enum-tag-scope-compat" fast \
    env CCC="$LCCC" bash tests/regression/check_enum_tag_scope_compat.sh
gate "enum-layout-semantics" fast \
    env CCC="$LCCC" bash tests/regression/check_enum_layout_semantics.sh
gate "ext-vector-one-lane-fp" fast \
    env CCC="$LCCC" bash tests/regression/check_ext_vector_one_lane_fp.sh
gate "strlen-embedded-nul" fast \
    env CCC="$LCCC" bash tests/regression/check_strlen_embedded_nul.sh
# Enum/enumerator types and `_Alignas(type)` member layout on x86-64 and i686:
# the corpus compares x86-64 output only, and both layouts differ on i686.
# The oracle is pinned to GCC 16 (`CC` defaults to the host `gcc` inside the
# scripts): enumerator types follow C23 6.7.2.2 only from GCC 13 onward, so a
# GCC 12 host oracle rejects the fixture's own static assertions.
gate "type-layout-fixtures" fast \
    env CCC="$LCCC" CC="${GCC:-gcc-16}" bash tests/regression/check_fixture_modes.sh \
    enum_type_semantics.c alignas_type_layout.c
# False type-dependent `_Static_assert`s (sizeof, _Generic, enumerator types)
# used to compile: the parser accepted whatever it could not evaluate.
gate "static-assert-sema" fast \
    env CCC="$LCCC" CC="${GCC:-gcc-16}" bash tests/regression/check_static_assert_sema.sh

gate "copy-alias-sizes" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_copy_alias_sizes.sh

gate "notype-code-routing" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_linker_notype_code.sh

gate "got64-old-spelling" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_got64_old_spelling.sh

gate "tls-pie-preemptible" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_tls_pie_preemptible.sh

# T5 (vellvm/vir transplant register): bounded fixpoint loops that exhaust
# their round budget are observable under CCC_TIME_PASSES, and the
# cross_pr_redteam.c backedge_pre cap hit (measured over 2,654 files) is the
# witness that the report is wired end to end.
gate "round-cap-observability" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_round_cap_observability.sh

# T6: the IR verifier (CCC_VERIFY_IR=abort) stays clean on a fixed 80-file subset.
gate "verify-ir-clean" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_verify_ir_clean.sh

# The dual-layout tie rule: the fixture still ties, the two arms differ, the
# default arm emits one fewer conditional branch, and both agree with the
# oracle on the program's output.
gate "layout-tie-rule" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_layout_tie_rule.sh

# T4/T7: mutation self-tests for the C reducer and the punning fuzzer's model.
gate "c-reduce-selftest" fast \
    python3 -m unittest discover -s tests/fuzz -p test_c_reduce.py

gate "punning-fuzz-selftest" fast \
    python3 -m unittest discover -s tests/fuzz -p test_punning_fuzz.py

gate "rmw-sib-folds" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_rmw_sib_folds.sh

gate "rmw-index-zext" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_rmw_index_zext_codegen.sh

gate "redundant-self-zext" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_redundant_self_zext.sh

gate "select-from-compare" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_select_from_compare.sh

gate "sparse-mask-updates" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_sparse_mask_updates.sh

gate "pointer-ring-indexing" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_pointer_ring_indexing.sh

gate "affine-loop-fold" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_affine_loop_fold.sh

gate "loop-downcount" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_loop_downcount.sh

gate "unsigned-affine-loop" fast \
    env CCC="$LCCC" bash tests/regression/check_unsigned_affine_loop.sh

# Differential -fwrapv witness for the exit-compare bound folds (signed
# affold + unsigned loop_bound) against a -O0 -fwrapv GCC reference.
gate "affine-wrapv-differential" fast \
    python3 tests/regression/check_affine_wrapv_differential.py --lccc target/fastbuild/lccc

# The FMA contract gate.  It is registered here because a regression test that
# nothing runs is the defect it was written to fix: the audit found the packed
# matmul arm contracting unconditionally, the fix shipped, and nothing pinned
# it -- so a later refactor could have removed the gate and every suite would
# still have gone green.
gate "fma-contract-gating" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_fma_gating.sh

# Destructive-FMA destination coalescing and stack FP parameters: the alias
# bodies and the stack-multiplicand body are executed symbolically and must
# return exactly the source expression (one FMA, no relay, no GPR bounce).
gate "fma-dest-coalesce" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_fma_dest_coalesce_codegen.sh

# Division-free vector remainder transitions: the vectorizer's signed trip
# bound `limit /s VF` must reach the backend strength-reduced (div_by_const
# reruns after the loop transforms), never as `idiv`.
gate "vector-remainder-codegen" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_vector_remainder_codegen.sh

# SSE4.1/AVX packed floor/ceil/trunc/nearbyint maps: every immediate mode,
# 128/256-bit ISA leg, scalar-tail length and signed-zero/NaN arm is pinned.
gate "vector-round-maps" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_vectorize_round_maps.sh

# Escaped locals under register pressure: constant-offset frame addresses are
# direct operands, never reloaded spilled pointers (zstd sequence-decoder shape).
gate "frame-gep-direct" fast \
    env CCC=target/fastbuild/lccc python3 tests/regression/check_frame_gep_direct.py

# Direct -c objects carry the same instructions as -S + GNU as (the private
# tight-loop marker must not disable peephole analyses in production builds).
gate "direct-assembly-parity" fast \
    env CCC=target/fastbuild/lccc python3 tests/regression/check_direct_assembly_parity.py

# Inliner: devirtualized per-iteration template predicates are inlined.
gate "inline-devirt-hot-leaf" fast \
    env CCC=target/fastbuild/lccc python3 tests/regression/check_inline_devirt_hot_leaf.py

# x86-64 shift counts: C masks that only feed hardware-masked counts are
# dropped, observable masks kept (zstd BIT_lookBits shape, kill-switch control).
gate "shift-count-demand" fast \
    env CCC=target/fastbuild/lccc python3 tests/regression/check_shift_count_demand.py

# BMI2 `_bzhi_u32/_u64`: one `bzhi` per opaque helper at x86-64-v3, none at
# x86-64, the builtin rejected without BMI2, results equal to GCC's.
gate "bmi2-bzhi" fast \
    env CCC=target/fastbuild/lccc GCC="${GCC:-gcc-16}" python3 tests/regression/check_bmi2_bzhi.py

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

# AUD-RATCHET-1: the audit's site counts are a ratchet, not a snapshot.  The
# scanner self-tests first (its tokenizer is what every number rests on), then
# --check refuses any metric that grew past scripts/audit_census_baseline.json:
# panic-class sites overall and in the user-input pipeline, `unsafe` sites,
# unannotated `#[ignore]`, functions over 300/500/1000 lines, and CCC_*/LCCC_*
# literals without documentation.  A count may only be re-baselined in the
# commit that adds the sites; lowering the baseline belongs to the commit that
# removes them.
gate "audit-census-selftest" fast \
    python3 scripts/audit_census.py --selftest
gate "audit-census-ratchet" fast \
    python3 scripts/audit_census.py --check

# The Expat differential is only a differential if both compilers ran the SAME
# tests under the SAME feature configuration.  `junit_check.py` compares testcase
# identity multisets and the CMake cache, and this gate runs its compiler-free
# contract so the rule is enforced even where no Expat build happens.
gate "expat-junit-contract" fast \
    python3 tests/workloads/expat-2.8.5/junit_check.py --self-test

# The AArch64 dispatcher ratchet is enforced twice -- by the committed goldens TSV
# and by an in-crate test that re-encodes every row -- and both enumerate the
# dispatcher by scanning Rust *text*.  A text scan that finds nothing prints a
# satisfied count, so the scanner's rules (region end by brace depth, arm heads
# invisible to comments, literals unable to fake a brace or a quote) are pinned
# here, toolchain-free, rather than only by the consumer they feed.  The
# "# arms:" header in the TSV is the latch between this scan and that consumer:
# if the two disagree about how many arms exist, both gates fail.
gate "dispatcher-region-contract" fast \
    python3 -m unittest discover -s tests/aarch64 -p 'test_*.py'

# A peephole walker may step over a `nop`, or over a directive that emits no
# data -- never over a bare `.byte`, which is machine code no mnemonic table can
# see.  48 of these skips existed in the two backends; the law now has one home
# (peephole_common::directive_emits_data) and this gate fails the build when a
# pass hand-rolls the old `is_nop() || kind == Directive` test again.
gate "peephole-directive-step" fast \
    python3 scripts/check_peephole_directive_step.py

# The implicit-operand table is what every fact-tracking pass consults; the
# fixture is the reviewer-facing statement of the architecture it encodes.
# This gate had been broken (and unregistered) since it was written.
gate "implicit-operand-matrix" fast \
    python3 scripts/check_implicit_operand_matrix.py

# A message that says "the ratchet is 151" while the gate still assigns 153 is
# not a typo: it is a reader told the limit lives somewhere it does not.  The
# verifiable half of a commit message -- named budgets, named paths -- is checked
# against the tree, so prose cannot drift ahead of the gates it describes.
gate "pr-claim-fidelity" fast \
    python3 scripts/check_pr_claim_fidelity.py

# Read-only integrity check; exclusive by default (not in scheduler whitelist).

gate "ci-resource-isolation" fast \
    python3 scripts/test_ci_resources.py

gate "ci-gate-parity" fast \
    python3 scripts/check_ci_gate_parity.py
gate "ci-asm-diff-parity-self-test" fast \
    python3 scripts/test_ci_gate_parity.py
# The splitter rewrites ci.yml.  A gate that lands in no shard silently stops
# running, and a gate that lands in two is paid for twice in a green run, so
# the transform itself is pinned (not just its output) on synthetic workflows.
gate "ci-lane-split-self-test" fast \
    python3 scripts/test_ci_lane_split.py

# The encdiff semantic-validation and asmdiff semantic-validation suites
# run AFTER the oracle provisioning below: their real-toolchain legs
# exercise the provisioned 2.47 pair (ASMDIFF_TEST_* / ENCDIFF_TEST_OBJDUMP
# channels) under LCCC_REQUIRE_PINNED_ORACLE=1, which makes a missing pin a
# hard failure instead of a silent skip — the ordering cannot rot, because
# running before provisioning now fails the gate itself. The invocation
# shapes (including these env channels) are pinned by
# check_invocation_contracts.

gate "gas-oracle-pair-self-test" fast \
    bash scripts/ensure_gas_247.sh --self-test

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

# Hot-web steal: opt-in diagnostics purity (no env, no output), a firing
# steal with its eviction cost on the scalar-facts witness, and a named
# cost-refusal on the all-hot fixture — the 5c.2 guard decisions made
# observable instead of re-derived by hand instrumentation.
gate "hot-web-steal-diagnostics" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_hot_web_steal_diagnostics.sh

# FP selects over relational compares whose arms are the compared operands
# lower to ONE maxsd/minsd/maxss/minss (second-operand-wins ties/NaNs, the
# C semantics exactly); ge/le stay unfolded. Bit-exact vs gcc-16 across
# -O0..-Os over the NaN/±0/SNaN corner grid.
gate "fp-select-minmax" fast \
    env CCC=target/fastbuild/lccc GCC=gcc-16 bash tests/regression/check_fp_select_minmax.sh

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

gate "asm-goto-phi-edge" fast \
    bash tests/regression/check_asm_goto_phi_edge.sh
gate "asm-goto-outputs" fast \
    bash tests/regression/check_asm_goto_outputs.sh
gate "i686-label-ref-safety" fast \
    bash tests/regression/check_i686_label_ref_safety.sh

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
# (idempotent, cached under ~/.cache; the tarball is sha256-pinned) by the
# same script hosted CI runs.
gate "asm-diff-oracle-gas-2.47" fast \
    bash scripts/ensure_gas_247.sh x86_64-linux-gnu

# GCC-compatible 16-byte vector_size i686 carriers, including the SSE-only
# and no-SSE boundaries, with typed shuffles across every compiler pairing.
# Mirrors the hosted ipa-lrs.yml step: GNU as 2.47 from the pin the gate
# above provisioned (the script refuses any other `as`), and the exact
# GCC 16.2 / Clang 23.1.4 oracles.
gate "i686-gnu-vector-abi" fast \
    env PATH="$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin:$PATH" CCC="$LCCC" \
        bash tests/regression/check_i686_gnu_vector_abi.sh

# The same provisioning for the AArch64 side, and it is load-bearing for
# every aarch64 gate below: they take their verdicts FROM a 2.47 `as`, and
# `$A64_AS` is resolved from the pinned prefix.  Without this gate the local
# mirror only *preferred* a cache entry nothing local was installing, so a
# clean box silently fell back to a distro `as` that disagrees with 1077
# rows 2.47 accepts.  Idempotent and tarball-cached like the x86 gate, so on
# a warm box it is a `test -x` away from free.
gate "a64-gas-provision" fast \
    bash scripts/ensure_gas_247.sh aarch64-linux-gnu

# The Arm A64 ISA XML package the documentation cross-check reads, same
# provisioning law as the GAS pair above: pinned SHA-256 tarball, only the
# 2026-09 member extracted, manifest re-checked after every install.  It is
# load-bearing for the aarch64-arm-doccheck gate far below, which validates
# every dispatcher golden against ARM DDI 0487 M.d's machine-readable
# encodings — older packages are NAK'd by the checker itself.
gate "arm-a64-xml-provision" fast \
    bash scripts/ensure_arm_a64_xml.sh

# The differential oracle's OWN contract suites, compiler-free exactly
# like the gates above but anchored to the REAL pinned pair: asmdiff's
# parser/oracle unit tests (the mocked failure modes — failed disassembler,
# empty listing, undecodable bytes, orphan fragments — plus the
# real-toolchain leg against the provisioned 2.47 pair) and encdiff's
# round-trip verdict tests. The integration gates prove the tools work on
# today's corpus; these prove the verdict machinery refuses the inputs an
# end-to-end green run can never exhibit. The pinned-tool env channels and
# LCCC_REQUIRE_PINNED_ORACLE=1 are part of the registered invocation
# contract: a leg that would silently skip or run against the runner's
# distro binutils is a contract violation, not a local convenience.
gate "encdiff-semantic-validation" fast \
    env ENCDIFF_TEST_OBJDUMP="$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/objdump" \
        LCCC_REQUIRE_PINNED_ORACLE=1 \
    python3 scripts/test_encdiff.py

gate "asmdiff-semantic-validation" fast \
    env ASMDIFF_TEST_AS="$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as" \
        ASMDIFF_TEST_OBJDUMP="$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/objdump" \
        LCCC_REQUIRE_PINNED_ORACLE=1 \
    python3 scripts/test_asmdiff.py

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
gate "sse34-clang-encoding" fast \
    python3 tests/regression/check_sse34_clang.py

gate "i686-tls-ie-relax" fast \
    bash tests/regression/check_i686_tls_ie_relax.sh

# 64-bit symbol-difference mov immediate (the mem_encrypt_boot.S idiom):
# the folded length must equal the label distance under either legal
# encoding fixpoint (imm32 or movabs).
gate "movabs-symdiff" fast \
    env CCC="$LCCC" bash tests/regression/check_movabs_symdiff.sh

# A `.symver`-bound indirect call must never fold to a direct/memory form
# (it would re-bind to the default version); the unversioned twin must fold.
gate "symver-indirect-fold" fast \
    env CCC_X64="$LCCC" bash tests/regression/check_symver_indirect_fold.sh
gate "i686-got32x-relax" fast \
    bash tests/regression/check_i686_got32x_relax.sh

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

# `narrow_lea_truncations` (`lea_narrow`): `leaq ADDR,%T` + `movl %Td,%Dd`
# becomes one `leal ADDR,%Dd`. Fires on 2 of the 49 benchmark programs and
# removes two of sixteen instructions in byte_load_reassembly's decode loop;
# the failure direction is a silent miscompile (an early write moving past a
# reader), so both the shape pin and the gcc execution differential run here.
gate "lea-truncation-narrowing" fast \
    bash tests/regression/check_lea_truncation_narrowing.sh

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

gate "slp-dword-constants" fast \
    bash tests/regression/check_slp_dword_constants.sh

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

# A store whose bytes the next iteration loads is a register carry, not a
# pack: BB-SLP declining it is what lets `loop_carried_forward` build
# the header phi (sha256_transform 12 % slower when packed).
gate "slp-loop-carried" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_slp_loop_carried_recurrence.sh

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

# Per-function __attribute__((target("avx2"))) must widen BB-SLP to ymm;
# an unattributed twin must stay xmm (ISA gate never becomes a global enable).
gate "target-attr-vec-isa" fast \
    bash tests/regression/check_target_attr_vec_isa.sh

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

# Unsigned word saturation: boundary values, scalar tails, overlap guards,
# signed/nonzero negative controls and the SSE2/AVX2/disabled-ISA contracts.
gate "u16-saturating-sub" fast \
    env CCC=target/fastbuild/lccc GCC="${GCC:-gcc-16}" \
        bash tests/regression/check_u16_saturating_sub.sh

# Byte/word saturating-subtract fold of the map vectorizer at every saturation
# boundary against GCC, plus the dword lane's correctness-only contract (x86
# has no packed dword saturating subtract to pin).
gate "vectorize-saturating-subtract" fast \
    env CCC=target/fastbuild/lccc REF="${GCC:-gcc-16}" \
        bash tests/regression/check_vectorize_saturating_subtract.sh

# Strict-FP reduction unrolling (the corpus' worst callgrind row,
# `reduction_vecreg`): `float sum(const float *, int)` must unroll 4x with the
# accumulation order intact, across every ISA/optimisation configuration, and
# the bits must match a sequential reference on association-sensitive payloads
# and every trip residue modulo 8.
gate "fp-reduction-unroll" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_fp_reduction_unroll.sh

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

# Fake-QEMU verdict controls: success, panic, missing markers/log and failed
# process. This checks the harness only, not whether an actual kernel boots.
gate "qemu-defconfig-smoke-verdict" fast \
    bash tests/regression/check_qemu_defconfig_smoke.sh

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

# Stack-slot dead stores: a 32-bit `leal` on a data-register %rbp used to bail
# never-read store elimination for the whole function, leaving `spill_deref`'s
# deleted reloads behind as dead stores in the hottest block (glibc_strstr:
# two of them, measured -1.0 % runtime).  The gate pins the structural contract
# on a fixture that reproduces the shape, and self-tests its checker.
gate "dead-stack-store" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_dead_stack_store.sh
# An empty inline-asm template assembles to nothing, so it must stage nothing:
# the old staging path reserved a scratch for every memory operand and spilled
# the address to the frame when the busy set was full, costing a store/load
# pair per iteration around every `asm volatile("" : "+m"(x))` barrier.
gate "asm-barrier-staging" fast \
    env CCC=target/fastbuild/lccc GCC_BIN=gcc-16 bash tests/regression/check_asm_barrier_no_spill.sh
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

# Best-effort EIP diagnostics for the known i686 small-vector runtime crash;
# the standard regression corpus remains the authoritative correctness gate.
gate "i686-small-vector-crash-diagnostic" fast \
    bash tests/regression/trace_vector_small_packed_wrappers_m32.sh

# `LCCC_IFCONV_ARM_BUDGET` is an A/B lever, and its contract is what keeps the
# numbers comparable: =1 tightens, off/full disable, an unparsable value is
# inert.  The gate also pins that the switch is resolved at the pipeline
# boundary (`src/passes/mod.rs`, which the env-read ratchet excludes) and not
# in the pass body -- reading it there is what put the ratchet one over budget
# when it landed.
gate "ifconv-arm-budget-switch" fast \
    env CCC="$LCCC" bash tests/regression/check_ifconv_arm_budget_switch.sh

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
# The gate's own argument parsing regressed once already: `--phase 1` stored the
# literal flag, selected nothing, ran no phase and still printed PASS.  That
# failure mode is invisible to the invariance gate below, which is `slow` and
# therefore absent from --fast, so the parser gets its own compiler-free gate
# that runs everywhere.
gate "peephole-whitespace-arg-selftest" fast \
    bash tests/regression/check_peephole_whitespace.sh --selftest
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

# The capability probe is what turns the Clang-C corpus from "almost every RUN
# filtered" (102 of 5 130 planned invocations executed) into a measured grid,
# and it is only worth anything if a flag the compiler ignores can never be
# reported as established.  The selftest drives the tool against a synthetic
# compiler that reproduces the contract the probe depends on -- including one
# that accepts a `-W`-spelled unknown option, one that ignores
# LCCC_STRICT_OPTIONS, and one that keeps tolerating `-Wno-<unknown>` under
# strict options; each mutation must abort the probe.  It also pins the
# per-invocation coverage rule against a weighted-sum misreading.  Pure
# Python, no toolchain, no corpus: fast.
gate "capability-probe-selftest" fast \
    python3 scripts/lccc_capability_probe.py --selftest

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

# Constant-folded late-dominating GEP: small external-TU observer and full
# zlib-derived table output. Both fixtures must pass at every optimization level.
gate "late-constant-gep" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_late_const_gep.sh

gate "arm-address-contract" fast \
    env CCC_ARM=target/fastbuild/lccc-arm python3 tests/regression/check_arm_address_contract.py

gate "arm-popcount-contract" fast \
    env CCC_ARM=target/fastbuild/lccc-arm python3 tests/regression/check_arm_popcount_contract.py

# Mandatory freestanding AArch64/RISC-V single-bit runtime checksum. The hosted
# shard runs it with --run; the local mirror compiles/links and executes when
# QEMU is present, so a missing emulator is an explicit skip rather than a
# silent pass. It must be a registered fast gate, not a stray command appended
# to the previous gate, or the shard-partition audit correctly rejects it.
gate "bit-width-runtime" fast \
    python3 tests/regression/check_bit_width_runtime.py

gate "popcount-loop-contract" fast \
    python3 tests/regression/check_popcount_loop_contract.py

# _Decimal64 indexed fold: the x86-64 indexed path must accept D64 on
# the load half, the store half, and the decider. Structural SIB-movsd
# assertions plus an integer-checksum gcc differential (mirrors the ci.yml
# step of the same script; ci-gate-parity fails if the two drift apart).
gate "decimal64-indexed-fold" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_decimal64_indexed_fold.sh
gate "affine-scaled-index-matrix" fast \
    env CCC="$LCCC" python3 tests/regression/check_affine_scaled_index_matrix.py

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
    gate "linker-harness-selftest" fast \
        python3 tests/linker/test_run_linker_helpers.py
    # GCC -fuse-ld=bfd does not discover the triplet-prefixed BFD 2.47 in the
    # GAS prefix; gotpcrel_edges needs it as an explicit newer-linker fallback.
    gate "linker-suite" fast env \
        LCCC_TEST_ASSEMBLER="$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as" \
        PATH="$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin:$PATH" \
        LCCC_REQUIRE_I386=1 \
        LCCC_RELOCS_TOOL="$HOME/.cache/lccc-kernel-tools/bin/relocs" \
        LCCC_BFD_LD="$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/x86_64-linux-gnu-ld.bfd" \
        python3 tests/linker/run_linker_tests.py --lccc target/fastbuild/lccc --strict
else
    echo "SKIP  linker fuzz + linker suite (target/fastbuild/lccc-ld not built)"
    SKIPPED=$((SKIPPED + 4))
fi

# Torture-corpus provisioning contract: stamp trust, integrity manifest,
# auto-repair, invalidate-before-replace (synthetic tarball, no network).
gate "ensure-gcc-torture-contract" fast \
    bash tests/regression/check_ensure_gcc_torture.sh

# Artifact and lane-mode contracts: a lane that downloads BARE compiler
# binaries must restore their executable bit (artifact uploads store content
# only), two payload shapes must not share an artifact name, and the GCC 17
# torture lane must install and verify its GCC 16.2 pin and pin its corpus
# revision. Static YAML/text analysis, no compiler.
gate "ci-artifact-modes" fast \
    python3 tests/regression/check_ci_artifact_modes.py

# The AArch64 differential oracle is the tool that found the four encoder
# defects above, and it is also the tool most likely to rot silently: a
# harness that has drifted out of agreement with GAS reports "OK" for
# everything and proves nothing. `--self-test-only` exercises the harness's
# own invariants (GAS/lccc agreement on known words, junk rejection,
# sparse-address extraction, the objdump canary) in a couple of seconds, so
# it runs on every --fast pass. The full sweep stays a manual, offline run.
#
# The gate is skipped, not failed, when the cross-binutils or the fastbuild
# lccc are absent: a dev box without aarch64 tooling is not a regression.
# The verdicts below come FROM the assembler, so *which* assembler decides
# whether the gate means anything: the distro `as` in this image does not know
# `tco`, `afgdtp0_el1` or `actlrmask_el1` and disagrees with 1077 rows that
# 2.47 accepts (measured), so the distro binary turns an agreement gate into a
# red one, so a distro fallback would fail loudly on rows that are not defects;
# these gates therefore run against the pinned `ensure_gas_247.sh
# aarch64-linux-gnu` triple (as, objdump, objcopy) and are skipped, not passed,
# without it.  The pin is spelled literally at each invocation:
# `scripts/check_ci_gate_parity.py` matches exact tokens, so no unpinned
# lookalike can satisfy the invocation contracts.
A64_PIN="$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin"
if [ -x "$A64_PIN/as" ] && [ -x "$A64_PIN/objcopy" ]; then
    A64_AS="$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin/as"
    A64_OBJCOPY="$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin/objcopy"
else
    A64_AS=""
    A64_OBJCOPY=""
fi
if [ -n "$A64_AS" ] && [ -n "$A64_OBJCOPY" ] && [ -x target/fastbuild/lccc ]; then
    gate "aarch64-oracle-selftest" fast \
        python3 scripts/aarch64_encoder_differential.py --self-test-only \
        --gas "$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin/as" \
        --objdump "$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin/objdump" \
        --objcopy "$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin/objcopy" \
        --lccc target/fastbuild/lccc
else
    echo "SKIP  aarch64-oracle-selftest (aarch64-linux-gnu binutils or" \
         "target/fastbuild/lccc not available)"
    SKIPPED=$((SKIPPED + 1))
fi

# The second aarch64 instrument, and the one that exists because the oracle
# above is structurally blind to a whole bug class: it feeds the assembler
# words objdump just decoded, so everything it hands over is a *valid*
# encoding by construction. `bic x0, x1, w2`, `mov sp, xzr` and
# `add x0, x1, x2, ror #3` are not encodings and can never appear there --
# yet accepting them silently is exactly as wrong as mis-encoding one. The
# matrix is a curated accept/reject table whose every expectation is GNU
# as's own verdict, re-checked against the cross assembler at gate time
# (10411 rows in two batched assembler runs -- 0.1 s measured with the pinned
# pair -- so the whole table is re-derived rather than sampled, with no
# compiler and no network), and it is what turned the review's "the operand
# checks look like class checks" into 38 executed counterexamples. It needs
# only `as`/`objcopy`, so it is skipped, not passed, when the cross-binutils
# are absent.
if [ -n "$A64_AS" ] && [ -n "$A64_OBJCOPY" ]; then
    gate "aarch64-operand-legality" fast \
        python3 scripts/aarch64_operand_legality_matrix.py --check \
        --as "$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin/as" \
        --objcopy "$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin/objcopy"
else
    echo "SKIP  aarch64-operand-legality (aarch64-linux-gnu binutils not" \
         "available)"
    SKIPPED=$((SKIPPED + 1))
fi

# The same table, asserted against *our* encoder. The gate above proves the
# table still says what GNU as says; this one proves we still do too -- a row
# can be correct while the encoder is wrong, which is the shape of every bug
# the matrix was written for. It needs `objcopy` (to read our own object) and
# the fastbuild binary, and is skipped, not passed, without them.
if [ -n "$A64_OBJCOPY" ] && [ -x target/fastbuild/lccc ]; then
    gate "aarch64-operand-legality-encoder" fast \
        python3 scripts/aarch64_operand_legality_matrix.py \
        --check-lccc target/fastbuild/lccc \
        --jobs 2 \
        --as "$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin/as" \
        --objcopy "$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin/objcopy"
else
    echo "SKIP  aarch64-operand-legality-encoder (aarch64-linux-gnu-objcopy or" \
         "target/fastbuild/lccc not available)"
    SKIPPED=$((SKIPPED + 1))
fi

# Documentation cross-check against the Arm source of truth: every golden
# row of tests/aarch64/dispatcher-goldens.tsv must satisfy the fixed-bit
# constraints of the 2026-09 A64 ISA XML (ARM DDI 0487 M.d).  The gate
# fails on a word that matches no encoding variant, on any release whose
# build-manifest is not 2026-09 (stale extracts cannot bless old
# encodings), and on an unparsable candidate file — the provision gate
# above guarantees the extract exists, so this never silently skips.
gate "aarch64-arm-doccheck" fast \
    python3 scripts/aarch64_doccheck_2026_09.py \
    --xml-dir "$HOME/.cache/arm-isa-a64-2026-09/ISA_A64_xml_A_profile-2026-09_md" \
    --goldens tests/aarch64/dispatcher-goldens.tsv

# The fourth aarch64 instrument, and the one that exists because the matrix is
# a table a HUMAN curated: it pins one row per slot shape, so it can only catch
# a defect in a spelling somebody thought of.  Every family whose rules are
# combinatorial needs the cross product as well, compared against the same
# pinned `as`: the twelve scalar SIMD&FP conversions (both forms, every width,
# every fixed-point bit count including both out-of-range ends), CASP's two
# register pairs (successor halves, the zero-register spelling, base classes),
# and add/sub's shifted and extended forms (kind x amount x width x extend
# kind).  Each of the three produced a measured defect -- a refused encoding we
# should accept, and an accepted spelling GNU as refuses -- and 2703 generated
# spellings re-prove it in ~20 s.
if [ -n "$A64_AS" ] && [ -n "$A64_OBJCOPY" ] && [ -x target/fastbuild/lccc ]; then
    gate "aarch64-family-sweeps" fast \
        python3 scripts/aarch64_family_sweeps.py \
        --lccc target/fastbuild/lccc \
        --as "$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin/as" \
        --objcopy "$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin/objcopy"
else
    echo "SKIP  aarch64-family-sweeps (pinned aarch64 binutils or" \
         "target/fastbuild/lccc not available)"
    SKIPPED=$((SKIPPED + 1))
fi

# The third aarch64 instrument: a *spelling* sweeper.  The matrix pins rows a
# human chose and the differential oracle only ever sees valid encodings, so
# neither reaches "the same instruction written a different way" --
# `ldrb xzr,[x0]` vs `ldrb wzr,[x0]`, `str q31,[x0]` vs `str v31,[x0]`,
# `[x0,x1]` vs `[x0,x1,lsl #0]`, the whole pstate-immediate domain.  411
# generated spellings, both verdicts, ~4 s.  Its four pinned divergences
# (GAS ignores trailing junk after the five-field raw sysreg spelling) are
# documented in the script and asserted here so a fifth one cannot hide.
if [ -n "$A64_AS" ] && [ -x target/fastbuild/lccc ]; then
    gate "aarch64-legality-probe" fast \
        python3 scripts/aarch64_legality_probe_differ.py \
        --lccc target/fastbuild/lccc \
        --as "$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin/as" \
        --objcopy "$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin/objcopy"
else
    echo "SKIP  aarch64-legality-probe (pinned aarch64 binutils or"
    echo "      target/fastbuild/lccc not available)"
    SKIPPED=$((SKIPPED + 1))
fi

# Directive-legality matrix, both halves.  The operand matrix above pins
# instruction-level verdicts; this one pins DIRECTIVE-level verdicts across
# the axes the word oracle is structurally blind to: section kind
# (.text/.data/custom-ax/custom-wa -- both targets), absurd align caps
# (1/5/100 pad bytes at @1 -- GAS 2.47 accepts them everywhere), the fill
# ceiling (2^62 x 4 accepted as size 0, x17 clamped, negative/zero size 0,
# oversized elements), the REPT count law (GAS's exact `excessive count`
# wording, the count==0 and negative-wrap corners, the 4 GiB boundary,
# `1<<64` evaluating to zero), and the x86-only jump-range reject class.
# `--check` re-derives every row from the pinned GNU as 2.47 pair (the
# table cannot rot silently); `--check-lccc` asserts the same rows against
# our own two front ends (a correct table cannot hide a wrong assembler).
# The pins are spelled literally at the invocation, like every gate here.
if [ -x "$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin/as" ] && \
   [ -x "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as" ]; then
    gate "directive-legality" fast \
        python3 scripts/directive_legality_matrix.py --check \
        --gas-a64 "$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin/as" \
        --gas-x64 "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as"
else
    echo "SKIP  directive-legality (pinned gas-2.47 pair not available)"
    SKIPPED=$((SKIPPED + 1))
fi
if [ -x target/fastbuild/lccc ]; then
    gate "directive-legality-lccc" fast \
        python3 scripts/directive_legality_matrix.py \
        --check-lccc target/fastbuild/lccc
else
    echo "SKIP  directive-legality-lccc (target/fastbuild/lccc not available)"
    SKIPPED=$((SKIPPED + 1))
fi

# Layout / directive bugs (`.p2align N,,M`) are invisible to the word oracle.
# Compile every tests/regression/arm_*.c to assembly, assemble it with the
# integrated assembler, and byte-compare .text against GNU as when present.
if [ -x target/fastbuild/lccc-arm ]; then
    # The system-register table the encoder resolves `mrs`/`msr` names through is
# generated from binutils' own `aarch64-sys-regs.def`, so it is only correct
# relative to a binutils release, and only complete relative to the .def it
# was generated from.  This gate holds three verdicts together at once: the
# .def file, the pinned GNU as, and the words this crate's encoder emits for
# every name in BOTH directions (1619 names, two assembler runs, ~4 s) -- the
# 115-name table it replaced had drifted from itself, and nothing was asking
# the assembler.  It needs the pinned binutils tarball for the .def file, so it
# is skipped, not passed, when that is absent (like the gates above).
if [ -n "$A64_AS" ] && \
   [ -f "${GAS_DL_DIR:-$HOME/.cache/dl}/binutils-2.47.tar.xz" ]; then
    gate "aarch64-sysreg-table" fast \
        python3 scripts/aarch64_sysreg_table.py --check \
        --as "$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin/as" \
        --objcopy "$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin/objcopy" \
        --lccc target/fastbuild/lccc
else
    echo "SKIP  aarch64-sysreg-table (pinned binutils tarball or aarch64"
    echo "      binutils not available)"
    SKIPPED=$((SKIPPED + 1))
fi

gate "arm-codegen-assembler-parity" fast \
        scripts/check_arm_codegen_assembles.sh
else
    echo "SKIP  arm-codegen-assembler-parity (target/fastbuild/lccc-arm not" \
         "available)"
    SKIPPED=$((SKIPPED + 1))
fi

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
gate "godbolt-explicit-local-selftest" fast \
    python3 tests/corpus/test_godbolt_explicit_local.py
# Offline static comparison; the hosted ICX lane additionally runs the same
# oracle with its authenticated Intel compiler and publishes paired timings.
gate "gap-local-assembly-oracle" fast \
    python3 scripts/godbolt.py compare --offline tests/regression/slp_independent_gap.c \
        --function gap_pairs --flags='-O2 -march=x86-64-v3' --local "$LCCC" --oracles '' \
        --local-ref "gcc=${GCC16:-gcc-16}"

# One CE cache now backs four tools, and every way it can be wrong is silent:
# a forgeable key serves one program's result for another's, a truncated
# record reads as a hit instead of a miss, and a cached rate-limit blip
# strands an oracle out of every future sweep. Pinned offline, no network.
gate "godbolt-cache-selftest" fast \
    python3 scripts/test_godbolt_cache.py

gate "generator-trace-selftest" fast \
    python3 tests/regression/test_generator_map_gate.py

gate "local-oracles-selftest" fast \
    python3 scripts/test_local_oracles.py

# Mirror the hosted cache integrity/identity contract; never a fast-lane download.
gate "toolchain-cache-restoration" slow \
    python3 scripts/restore_toolchain_cache.py --prefix "$HOME/.cache/lccc/reviewed-oracles"

gate "icx-provision-selftest" fast \
    python3 scripts/test_icx_provision.py

# MS-11: glibc `make check` triage classifier (miscompile / unsupported /
# environment). Fixture-driven, offline.
gate "glibc-triage-selftest" fast \
    python3 scripts/test_glibc_check_triage.py
gate "glibc-triage-cli-selftest" fast \
    python3 scripts/glibc_check_triage.py --self-test
gate "glibc-build-guards-selftest" fast \
    bash tests/workloads/glibc-2.44/test_build_guards.sh

# MS-01: Compiler Explorer oracle delta gate logic (metrics, tolerance,
# baseline handling). Networked surveys are opt-in research, not merge gates.
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

# The declared rust-version floor (Cargo.toml, 1.97.0) is a measurement only
# if something rebuilds the tree on that toolchain: the hosted `msrv` job
# does, and this gate is its local mirror (check_ci_gate_parity.py requires
# the pair). Needs the 1.97.0 toolchain: through rustup where available,
# otherwise directly when the active rustc already IS the declared floor
# (restored sandboxes install a plain dist toolchain without rustup), or from a
# pinned dist toolchain unpacked outside rustup -- LCCC_MSRV_TOOLCHAIN, else the
# tag-cache prefix the restored sandboxes build at $HOME/.cache/lccc/msrv-1.97.0.
# rustup stays canonical and is tried first, so hosted runs are unchanged; the
# pinned prefix is verified against the declared floor before it is trusted, so
# a stale or wrong directory cannot turn this gate green: it stays fail-closed.
msrv_check_gate() {
    if command -v rustup >/dev/null 2>&1; then
        cargo +1.97.0 check --locked
        return
    fi
    if rustc --version | grep -q '^rustc 1\.97\.0'; then
        cargo check --locked
        return
    fi
    local prefix="${LCCC_MSRV_TOOLCHAIN:-$HOME/.cache/lccc/msrv-1.97.0}"
    local pinned=""
    if [ -x "$prefix/bin/rustc" ]; then
        pinned="$("$prefix/bin/rustc" --version 2>/dev/null)"
    fi
    case "$pinned" in
    rustc\ 1.97.0*)
        PATH="$prefix/bin:$PATH" CARGO_TARGET_DIR="${MSRV_TARGET_DIR:-target/msrv-cargo}" \
            cargo check --locked
        return
        ;;
    esac
    if [ -n "$pinned" ]; then
        echo "msrv-check: $prefix is not the declared floor ($pinned)" >&2
        return 1
    fi
    echo "msrv-check: need rustc 1.97.0 (rustup missing, active $(rustc --version))" >&2
    return 1
}
gate "msrv-check" fast msrv_check_gate

# Test and Clippy gates use the same resource policy, independently in
# subshells. CI_LOCAL_JOBS / CARGO_BUILD_JOBS override automatic parallelism;
# explicit debug/incremental preferences affect only those respective knobs.
clippy_gate() (
    local flags=""
    [ -r target/lccc-rustflags ] && flags="$(cat target/lccc-rustflags)"
    configure_cargo_gate_resources || return
    # RUSTFLAGS reuses the build gate's resolved flags (see cargo_test_repeated):
    # without them clippy recompiles the whole crate under a different flag set.
    #
    # The wrapper (not bare `cargo clippy`) is load-bearing: the crate root
    # carries `#![allow(unknown_lints)]` so that a Clippy newer than the
    # declared MSRV cannot break `-D warnings` on a lint name that does not
    # exist yet, and the wrapper re-probes every name in the policy list under
    # `#![deny(unknown_lints)]` to keep typos from hiding behind that allow.
    cargo clippy --version || return
    RUSTFLAGS="$flags" python3 scripts/check_clippy_policy.py
)

gate "clippy" fast clippy_gate

# Assertion-enabled units and corpus share an isolated target directory and
# identical flags. Never overwrite/rebuild the normal compiler after this gate.
regression_corpus_dbgassert() {
    local flags="" rc=0
    [ -r target/lccc-rustflags ] && flags="$(cat target/lccc-rustflags)"
    CARGO_TARGET_DIR=target/ci-debug CARGO_INCREMENTAL=0 RUSTFLAGS="$flags" cargo build --profile fastbuild \
        --bin lccc --locked -j 2 "${DBGASSERT_CONFIG[@]}" || return 1
    CCC_VALIDATE_SSA=1 python3 tests/regression/run_regression.py \
        --lccc target/ci-debug/fastbuild/lccc -j 2 --gcc gcc-16 \
        --json target/regression-dbgassert-results.json || rc=1
    return $rc
}
gate "regression-corpus-debug-assertions" slow regression_corpus_dbgassert

# ------------------------------------------------------------------ done ---
ci_gate_drain
hr
echo "SUMMARY: ${PASSED} passed, ${#FAILED[@]} failed, ${SKIPPED} skipped"
if [ ${#FAILED[@]} -gt 0 ]; then
    for f in "${FAILED[@]}"; do echo "  FAILED: $f"; done
    exit 1
fi
echo "ALL GATES GREEN"
if [ -n "$ONLY" ] || [ "$SKIP_MSRV" = 1 ]; then
    echo "(explicit gate selection: partial run, no pass stamp)"
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
                # Owner policy: a local --fast (or --slow) pass is the accepted
                # local gate. GitHub CI runs every gate on the PR.
                mode=$half
                echo "stamp: mode=$half (local gate; GitHub CI runs the rest)" >&2
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
