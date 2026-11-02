#!/usr/bin/env bash
# Bounded parallelism for audited compiler-free, isolated gates only. Unknown
# gates are exclusive barriers. Never run two Cargo builds or oracle installers
# together on the constrained local host. Source after initializing counters.
CI_GATE_LIMIT=${CI_LOCAL_GATE_JOBS:-2}
if [[ ! $CI_GATE_LIMIT =~ ^[12]$ ]]; then
    echo 'CI_LOCAL_GATE_JOBS must be 1 or 2' >&2
    exit 2
fi
CI_GATE_PIDS=()
CI_GATE_NAMES=()
CI_GATE_LOGS=()
CI_GATE_LOG_DIR=$(mktemp -d "${TMPDIR:-/tmp}/lccc-ci-gates.XXXXXXXX") || exit 1
ci_gate_record() {
    local name=$1 rc=$2
    if ((rc == 0)); then
        PASSED=$((PASSED + 1))
    else
        FAILED+=("$name")
    fi
}
ci_gate_run() {
    local name=$1 start rc; shift
    start=$SECONDS
    printf '\nGATE  %s\n' "$name"
    if "$@"; then rc=0; else rc=$?; fi
    if ((rc == 0)); then
        printf 'PASS  %s  (%ss)\n' "$name" "$((SECONDS - start))"
    else
        printf 'FAIL  %s  (%ss; exit=%s)\n' "$name" "$((SECONDS - start))" "$rc"
    fi
    return "$rc"
}
ci_gate_reap() {
    local rc=0
    wait "${CI_GATE_PIDS[0]}" || rc=$?
    cat "${CI_GATE_LOGS[0]}"
    rm -f "${CI_GATE_LOGS[0]}"
    ci_gate_record "${CI_GATE_NAMES[0]}" "$rc"
    CI_GATE_PIDS=("${CI_GATE_PIDS[@]:1}")
    CI_GATE_NAMES=("${CI_GATE_NAMES[@]:1}")
    CI_GATE_LOGS=("${CI_GATE_LOGS[@]:1}")
}
ci_gate_drain() {
    while ((${#CI_GATE_PIDS[@]})); do ci_gate_reap; done
}
ci_gate_parallel_safe() {
    # Deliberate whitelist, not '*selftest*': future gates default to exclusive.
    case "$1" in
        no-conflict-markers|ci-workflow-shell|script-imports|volatile-destructuring-selftest|volatile-destructuring|benchmark-codegen-registry|gcc-provisioning-mocks|swap-provisioning-mocks|rust-toolchain-selector|linker-oracle-versions|i686-run-policy|ci-gate-parity|ci-asm-diff-parity-self-test|gas-oracle-pair-self-test|edg-changes-miner-selftest|edg-corpus-miner-selftest|corpus-index-integrity|edg-changes-artifact-integrity) return 0 ;;
        *) return 1 ;;
    esac
}
ci_gate_submit() {
    local name=$1 log rc=0; shift
    if ((CI_GATE_LIMIT > 1)) && ci_gate_parallel_safe "$name"; then
        if ((${#CI_GATE_PIDS[@]} >= CI_GATE_LIMIT)); then ci_gate_reap; fi
        log=$(mktemp "$CI_GATE_LOG_DIR/gate.XXXXXXXX") || return 1
        (trap - EXIT INT TERM; ci_gate_run "$name" "$@") >"$log" 2>&1 &
        CI_GATE_PIDS+=("$!")
        CI_GATE_NAMES+=("$name")
        CI_GATE_LOGS+=("$log")
    else
        ci_gate_drain
        ci_gate_run "$name" "$@" || rc=$?
        ci_gate_record "$name" "$rc"
    fi
}
ci_gate_cleanup() {
    # On normal exit every gate is already drained. On interrupt, wait for
    # outstanding isolated checks before removing their logs or returning.
    ci_gate_drain
    rmdir "$CI_GATE_LOG_DIR"
}
trap ci_gate_cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
