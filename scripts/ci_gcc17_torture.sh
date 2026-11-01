#!/usr/bin/env bash
# Run the GCC 17 development gcc.c-torture compile + execute legs for one x86
# target. The Git checkout is revision-pinned by ensure_gcc_torture_git.py;
# both reports include that immutable commit and partial checkpoints are
# written throughout each run.
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
gcc=${GCC_BIN:-$(command -v gcc-16 || command -v gcc || true)}
[[ -n $gcc ]] || { echo "ci_gcc17_torture: GCC reference compiler not found" >&2; exit 2; }
# A missing file and an unexecutable one are different defects with different
# fixes, and both used to print the same "not executable" line: a lane that
# downloads the compiler as an artifact always arrives 0644 (zip transport keeps
# no Unix mode), so the message has to name the actual remedy.
[[ -e $compiler ]] || { echo "ci_gcc17_torture: compiler not found: $compiler" >&2; exit 2; }
[[ -x $compiler ]] || { echo "ci_gcc17_torture: compiler is not executable (chmod +x it; artifact transport drops the mode): $compiler" >&2; exit 2; }
[[ -e $linker ]] || { echo "ci_gcc17_torture: standalone linker not found: $linker" >&2; exit 2; }
[[ -x $linker ]] || { echo "ci_gcc17_torture: standalone linker is not executable (chmod +x it; artifact transport drops the mode): $linker" >&2; exit 2; }

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

provision=(python3 "$repo_root/scripts/ensure_gcc_torture_git.py" --dest "$suite_root")
if [[ -n ${GCC17_TORTURE_REF:-} ]]; then
    provision+=(--ref "$GCC17_TORTURE_REF")
fi
"${provision[@]}"

suite_base="$suite_root/gcc/testsuite/gcc.c-torture"
execute_suite="$suite_base/execute"
compile_suite="$suite_base/compile"
[[ -d $execute_suite && -d $compile_suite ]] || {
    echo "ci_gcc17_torture: provisioned checkout lacks execute/compile suites" >&2
    exit 2
}

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
    printf '\n== GCC 17 torture: arch=%s mode=%s flags=%s GCC=%s%s ==\n' \
        "$arch" "$mode" "$flags" "$gcc" "${filter:+ filter=$filter}"
    local rc=0
    if "${command[@]}"; then
        return 0
    else
        rc=$?
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
