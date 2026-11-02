#!/usr/bin/env bash
# Run the GCC 17 development gcc.c-torture compile + execute legs for one x86
# target. The Git checkout is revision-pinned by ensure_gcc_torture_git.py;
# both reports include that immutable commit and partial checkpoints are
# written throughout each run.
#
# Two pins decide what the reports mean, and both are asserted here rather than
# assumed from the job environment:
#
#   * the CORPUS revision defaults to c94b688a..., the trunk revision every
#     number in engineering/FOLLOWUP-2026-10-31-gcc17-torture-x86-fixes-ci.md
#     was measured against. Following `refs/heads/master` on every main push
#     turns this lane red whenever upstream adds a torture case LCCC does not
#     handle yet -- a real campaign signal, but not a regression in the tree
#     that pushed. GCC17_TORTURE_REF (or the dispatch `ref` input) overrides,
#     including `GCC17_TORTURE_REF=refs/heads/master` to track trunk again.
#   * the REFERENCE COMPILER is GCC 16.2 exactly (GCC_TORTURE_GCC_VERSION), as
#     delivered by scripts/ci_install_gcc_oracle.sh from the tag cache; the
#     distribution's rolling `gcc-16` is 16.0.1 on the runner image and would
#     silently re-base every per-test identity on a version no document claims.
#
# Verdict policy. `x86_gcc_torture.py` exits 0 clean, 1 on candidate failures and
# 2 when the RUN is not evidence (all-skipped, reference preflight, mid-flight
# input drift). A full corpus has candidate failures today -- the campaign record
# inventories 49 x86-64 / 48 i686 compile-leg gaps and known gap 1 says the
# per-test list, not a collapsed bucket, is the deliverable -- so exit 1 is not
# automatically a regression, while exit 2 always is. GCC_TORTURE_ADVISORY_FAILURES=1
# (what the hosted lane sets) therefore publishes the inventory and keeps the
# lane green on exit 1, and stays fatal on exit 2. Local runs are strict by
# default. Flip the lane to strict once a baseline inventory is checked in; that
# is the follow-up recorded in the campaign document.
set -Eeuo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

arch=${1:?usage: ci_gcc17_torture.sh x86_64|i686}
case "$arch" in
    x86_64)
        compiler=${LCCC_X86_64:-$repo_root/target/fastbuild/lccc}
        ;;
    i686)
        compiler=${LCCC_I686:-$repo_root/target/fastbuild/lccc-i686}
        ;;
    *)
        echo "ci_gcc17_torture: unsupported architecture '$arch' (choose x86_64 or i686)" >&2
        exit 2
        ;;
esac
linker=${LCCC_LD:-$repo_root/target/fastbuild/lccc-ld}

# A downloaded artifact arrives at mode 0644: actions/upload-artifact stores
# content, not permissions. This driver is the first thing to touch the
# binaries, so its guard has to say WHICH failure it is -- a missing file is a
# broken artifact, a non-executable one is a missing `chmod +x` in the
# consuming job. Collapsing both into "not executable" (and letting the shell
# report exit 2 with no output) is what made this lane's first red run look
# like a compiler problem instead of a packaging one.
require_executable() {
    local what=$1 path=$2
    if [[ ! -e $path ]]; then
        echo "ci_gcc17_torture: $what not found: $path (was the fastbuild artifact downloaded?)" >&2
        exit 2
    fi
    if [[ ! -x $path ]]; then
        echo "ci_gcc17_torture: $what is not executable: $path" >&2
        echo "ci_gcc17_torture: artifact uploads store content only; the consuming job must chmod +x it" >&2
        exit 2
    fi
}
require_executable compiler "$compiler"
require_executable "standalone linker" "$linker"

# Resolve the reference compiler to an absolute path, so the banner and the
# per-leg reports name the binary that actually ran rather than a PATH entry.
gcc_requested=${GCC_BIN:-gcc-16}
gcc=$(command -v "$gcc_requested" 2>/dev/null || true)
[[ -n $gcc ]] || {
    echo "ci_gcc17_torture: reference compiler '$gcc_requested' not found on PATH" >&2
    exit 2
}
gcc_version=$("$gcc" -dumpfullversion 2>/dev/null || true)
if [[ -n ${GCC_TORTURE_GCC_VERSION:-} ]]; then
    case $gcc_version in
        "$GCC_TORTURE_GCC_VERSION" | "$GCC_TORTURE_GCC_VERSION".*) ;;
        *)
            echo "ci_gcc17_torture: reference compiler $gcc reports '${gcc_version:-no version}'," \
                "but this lane pins GCC $GCC_TORTURE_GCC_VERSION" >&2
            echo "ci_gcc17_torture: install the exact release from the tag cache" \
                "(scripts/ci_install_gcc_oracle.sh --version $GCC_TORTURE_GCC_VERSION) instead of" \
                "relying on the distribution's rolling gcc-16" >&2
            exit 2
            ;;
    esac
fi

# Corpus revision: pinned by default, overridable to track trunk (see the
# header). The full SHA is required by the provisioner's own --ref contract.
corpus_ref=${GCC17_TORTURE_REF:-c94b688a83fc84015c6fab8ab0ff142051598fc8}

suite_root=${GCC17_TORTURE_ROOT:-$HOME/.cache/lccc-gcc17}
results=${GCC_TORTURE_RESULTS_DIR:-$repo_root/artifacts/gcc17-torture/$arch}
flags=${GCC_TORTURE_FLAGS:--O2}
jobs=${GCC_TORTURE_JOBS:-2}
compile_timeout=${GCC_TORTURE_COMPILE_TIMEOUT:-180}
run_timeout=${GCC_TORTURE_RUN_TIMEOUT:-10}
filter=${GCC_TORTURE_FILTER:-}
[[ $jobs =~ ^[1-9][0-9]*$ ]] || { echo "ci_gcc17_torture: invalid GCC_TORTURE_JOBS '$jobs'" >&2; exit 2; }
[[ $compile_timeout =~ ^[1-9][0-9]*$ ]] || { echo "ci_gcc17_torture: invalid GCC_TORTURE_COMPILE_TIMEOUT '$compile_timeout'" >&2; exit 2; }
[[ $run_timeout =~ ^[1-9][0-9]*$ ]] || { echo "ci_gcc17_torture: invalid GCC_TORTURE_RUN_TIMEOUT '$run_timeout'" >&2; exit 2; }
mkdir -p "$results"

provision=(python3 "$repo_root/scripts/ensure_gcc_torture_git.py"
    --dest "$suite_root" --ref "$corpus_ref")
"${provision[@]}"

suite_base="$suite_root/gcc/testsuite/gcc.c-torture"
execute_suite="$suite_base/execute"
compile_suite="$suite_base/compile"
[[ -d $execute_suite && -d $compile_suite ]] || {
    echo "ci_gcc17_torture: provisioned checkout lacks execute/compile suites" >&2
    exit 2
}

advisory=${GCC_TORTURE_ADVISORY_FAILURES:-0}
[[ $advisory =~ ^[01]$ ]] || { echo "ci_gcc17_torture: invalid GCC_TORTURE_ADVISORY_FAILURES '$advisory'" >&2; exit 2; }
status=0
run_leg() {
    local mode=$1 suite=$2 report=$3 failure_log=$4
    local -a command=(python3 "$repo_root/scripts/x86_gcc_torture.py"
        --arch "$arch" --mode "$mode" --suite "$suite"
        --lccc "$compiler" --lccc-ld "$linker" --gcc "$gcc"
        --flags="$flags" --jobs "$jobs" --compile-timeout "$compile_timeout"
        --run-timeout "$run_timeout" --json "$report"
        --failure-log "$failure_log")
    if [[ -n $filter ]]; then
        command+=(--filter "$filter")
    fi
    printf '\n== GCC 17 torture: arch=%s mode=%s flags=%s GCC=%s (%s) corpus=%s%s ==\n' \
        "$arch" "$mode" "$flags" "$gcc" "${gcc_version:-unknown}" "$corpus_ref" \
        "${filter:+ filter=$filter}"
    local rc=0
    if "${command[@]}"; then
        return 0
    else
        rc=$?
    fi
    if (( rc == 1 )) && [[ $advisory == 1 ]]; then
        # Exit 1 is "candidate failures", i.e. cases LCCC fails that the
        # reference accepts. Those are inventoried, not hidden: both files below
        # are uploaded as the lane's artifact.
        local note="GCC 17 torture $arch/$mode: candidate failures (runner exit 1);"\
" inventory in $report and $failure_log; the lane reports them until a"\
" checked-in baseline makes them blocking"
        echo "ci_gcc17_torture: $note" >&2
        if [[ -n ${GITHUB_ACTIONS:-} ]]; then
            printf '::warning title=%s::%s\n' "GCC 17 torture $arch/$mode" "$note"
        fi
        return 0
    fi
    echo "ci_gcc17_torture: $arch/$mode failed (exit $rc); see $report and $failure_log" >&2
    status=1
}

# Run both legs even if one fails, preserving the second report for diagnosis.
run_leg execute "$execute_suite" "$results/$arch-execute.json" \
    "$results/$arch-execute.failures.txt"
run_leg compile "$compile_suite" "$results/$arch-compile.json" \
    "$results/$arch-compile.failures.txt"
exit "$status"
