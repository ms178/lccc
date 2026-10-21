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
#   ./scripts/ci_local.sh --fast       # skip ONLY the six slow gates:
#                                      #   cargo-test-debug-assertions,
#                                      #   regression-corpus-ssa,
#                                      #   benchmark-output-oracle,
#                                      #   bounded-callgrind-corpus,
#                                      #   peephole-whitespace-invariance,
#                                      #   regression-corpus-debug-assertions
#                                      # rustfmt and clippy ALWAYS run — they
#                                      # are the CI lint jobs, and skipping
#                                      # them locally is how a red PR ships.
#   ./scripts/ci_local.sh --slow       # ONLY the six slow gates (plus the
#                                      # build).  If a --fast stamp for the SAME
#                                      # tree exists, a green --slow run upgrades
#                                      # it to mode=full: fast + slow on one tree
#                                      # is every gate GitHub runs.
#   ./scripts/ci_local.sh --only NAME  # a single gate, substring match
#
# A --fast pass is NOT CI-equivalent: GitHub runs all six slow gates on every
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

# ---------------------------------------------------------------- build ----
gate "build" fast ./scripts/build_lccc_fast.sh

if [ ! -x "$LCCC" ]; then
    echo "FATAL: $LCCC was not produced; every later gate would be vacuous." >&2
    exit 1
fi

gate "driver-option-contract" fast \
    env CCC="$LCCC" bash tests/regression/check_driver_option_contract.sh
gate "default-isa-policy" fast \
    env CCC="$LCCC" bash tests/regression/check_default_isa_policy.sh
gate "cas16-lowering" fast \
    env CCC="$LCCC" bash tests/regression/check_cas16_lowering.sh
gate "atomic16-runtime" fast \
    env CCC="$LCCC" bash tests/regression/check_atomic16_runtime.sh
gate "fno-builtin-fold-contract" fast \
    env CCC="$LCCC" bash tests/regression/check_fno_builtin_folds.sh

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

# ~97 compiles per arm plus the runs: seconds, not minutes -- and it is the
# only gate that exercises the historical crash/miscompile corpus end to end.
gate "cpu-supports-runtime" fast \
    env CCC="$LCCC" bash tests/regression/check_cpu_supports_runtime.sh

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

gate "generator-map" fast \
    env CCC="$LCCC" bash tests/regression/check_generator_map.sh

# Register-offset symbol folds (`Add(symbol, register)` -> `sym(%off,%idx)`):
# the sliding-window shape `const u8 *m = window + cur; m[i]` must fold the
# symbol into the memory operand where the derived pointer has no other use,
# must shrink the function when it does, must leave the RA-01 kernel dormant
# where it cannot pay (measured: identical body), must agree with GCC on
# every program's output, and must refuse PIC (no third SIB slot).
gate "reg-off-sym-fold" fast \
    env CCC="$LCCC" bash tests/regression/check_reg_off_sym_fold.sh

# Array-bound contract: a bad bound is diagnosed (never wrapped), a byte size
# that cannot exist is rejected instead of wrapping into a bogus section, and a
# huge `.bss` object is emitted sparsely under a hard RLIMIT_AS rather than
# being materialised and OOM-killed. Driven under that limit so the "constant
# memory" half is a real requirement, not a hope.
gate "array-bound-contract" fast \
    python3 scripts/check_array_bound_contract.py --lccc "$LCCC"

# UB-exploitation invariant (E22): `-fno-strict-aliasing`, `-fwrapv` and
# `-fno-strict-overflow` are tolerated because this optimizer exploits no
# type-based aliasing or signed-overflow poison. That is a precondition, not
# a property of the universe: the day such a concept lands in src/, the
# tolerated spellings become silent miscompiles. The tripwire names them.
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

gate "gcc-provisioning-mocks" fast \
    python3 tests/regression/check_ci_gcc16_provisioning.py

gate "swap-provisioning-mocks" fast \
    python3 tests/regression/check_ci_swap_provisioning.py

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
# pinned Clang 23.1.3 oracle and linked in all four combinations; shapes
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
# The same probe under the AVX / AVX-512F ABI level (x86-64 only): a named
# ext vector of up to the native register width is ONE %ymm / %zmm for
# Clang, the carrier lccc shares with the vector_size ABI (AUD-VABI-2), so
# every leg must agree, returns included.
gate "ext-vector-abi-avx" fast \
    env CCC="$LCCC" bash tests/regression/check_ext_vector_abi_avx.sh

# One-lane ext_vector carriers: caller-side marshalling (register class and
# width at -O0/-O1/-O2), FP/int lane-1 carrier correctness, and narrow
# add/sub/mul/neg with signedness, A/B across the small-slot configuration.
# These were orphaned (not wired into any gate) until this entry.
gate "ext-vector-callsite-abi" fast \
    env CCC="$LCCC" bash tests/regression/check_ext_vector_callsite_abi.sh
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
gate "ivsr-reproducers" fast \
    env CCC="$LCCC" bash tests/regression/check_ivsr_reproducers.sh


# AUD-ICE-1/AUD-DIAG-1: a malformed `__builtin_shuffle` mask must produce a
# diagnostic, not a Rust panic (and the valid forms must still lower).  The
# gate is oracle-checked against gcc, so it cannot pass by rejecting more than
# GCC does.
gate "shuffle-diagnostics" fast \
    env CCC="$LCCC" bash tests/regression/check_shuffle_diagnostics.sh

# Clang ext_vector_type component access against the Clang 23.1.3 oracle:
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

gate "loop-invert-census" fast \
    python3 scripts/loop_invert_census.py --lccc "$LCCC" --quiet

gate "fuzz-harness-tests" fast \
    python3 -m unittest discover -s tests/fuzz -p 'test_*.py'

gate "fuzz-engine-wiring" fast \
    python3 scripts/fuzz_diff.py --check-engines

gate "differential-fuzz-smoke" fast \
    python3 scripts/fuzz_diff.py --engine synthetic --count 4 \
        --lccc "$LCCC" --refs gcc --opts=-O0,-O2 --seed 20260906

# Missed-alarm pairs (scripts/mutpair.py): execution-filtered known-inequivalent
# / known-equivalent near-miss pairs in one TU, plus the harness's own
# negative control (a fake optimiser must be caught) in the unit tests.
gate "mutpair-harness-tests" fast \
    sh -c 'python3 scripts/mutpair.py --self-test && python3 -m unittest scripts.test_mutpair -v'

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
gate "tls-cross-models" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_tls_cross_models.sh

# Hermetic oracle self-tests for the TLS runtime comparison harness
# (no compiler required). Keep exactly one registration beside its rationale.
gate "tls-runtime-oracle" fast bash tests/regression/check_tls_runtime_oracle.sh

gate "volatile-reduction" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_volatile_reduction.sh

gate "llvm-missed-inventory-tests" fast python3 scripts/test_llvm_missed_inventory.py
gate "llvm-missed-inventory-optimized-tests" fast python3 -O scripts/test_llvm_missed_inventory.py
gate "llvm-missed-zero-choice" fast python3 tests/regression/check_llvm_missed_zero_choice.py
gate "llvm-integer-observations" fast python3 tests/regression/check_llvm_integer_observations.py
gate "llvm-bit-demand" fast python3 tests/regression/check_llvm_bit_demand.py
gate "llvm-scalar-facts" fast python3 tests/regression/check_llvm_scalar_facts.py
gate "llvm-guarded-folds" fast python3 tests/regression/check_llvm_guarded_folds.py
gate "llvm-word-counts" fast python3 tests/regression/check_llvm_word_counts.py
# Dual-layout metric pair: assembly identity across serial/auto/forced, kill
# switch, host fallback and pair liveness. Mirrors the hosted step.
gate "peephole-pair-parallel" fast python3 tests/regression/check_peephole_pair_parallel.py
gate "oracle-diagnostics" fast python3 scripts/test_ci_oracle_diagnostics.py

gate "tls-model-option" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_tls_model_option.sh

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

# T4/T7: mutation self-tests for the C reducer and the punning fuzzer's model.
gate "c-reduce-selftest" fast \
    python3 -m unittest discover -s tests/fuzz -p test_c_reduce.py

gate "punning-fuzz-selftest" fast \
    python3 -m unittest discover -s tests/fuzz -p test_punning_fuzz.py

gate "rmw-sib-folds" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_rmw_sib_folds.sh

gate "rmw-index-zext" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_rmw_index_zext_codegen.sh

gate "select-from-compare" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_select_from_compare.sh

gate "affine-loop-fold" fast \
    env CCC=target/fastbuild/lccc bash tests/regression/check_affine_loop_fold.sh

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
    gate "linker-suite" fast env \
        LCCC_TEST_ASSEMBLER="$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as" \
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
# (restored sandboxes install a plain dist toolchain without rustup).
msrv_check_gate() {
    if command -v rustup >/dev/null 2>&1; then
        cargo +1.97.0 check --locked
    elif rustc --version | grep -q '^rustc 1\.97\.0'; then
        cargo check --locked
    else
        echo "msrv-check: need rustc 1.97.0 (rustup missing, active $(rustc --version))" >&2
        return 1
    fi
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
