#!/usr/bin/env bash
# Four invariants this tree used to violate, each checkable by grep — so each can
# also silently come back unless something fails when it does.
#
# 1. No test mutates the process environment except through the RAII guard in
#    src/test_support.rs.  Five deferred-audit markers stood in for that audit;
#    the audit found the premise false (cargo runs `#[test]` functions on a
#    thread pool, and glibc's `setenv` may reallocate the `environ` array another
#    thread's `getenv` is walking — which is why edition 2024 made the calls
#    `unsafe`).  A uniquely named variable protects the VALUE, not the array.
#    The guard is the answer: one process-wide window, restored in `Drop` so a
#    failing assertion cannot leak the variable into the tests after it.
#
# 2. The two passes this tree migrated completely (vec_interleave, vec_load_sink)
#    contain no environment read at all: `run_passes` resolves their config once
#    and they read per-thread state.  Per call they used to do four `environ`
#    scans and allocate four strings per function, one of them (`CCC_VEC_
#    INTERLEAVE`) inside the candidate loop.
#
# 3. The pass pipeline as a whole does not GROW its environment reads.  This is a
#    ratchet, not a ban, and the number is measured rather than hoped for: 157
#    call sites across 41 files at the time of writing, 123 of them the
#    `String`-allocating `env::var` form, against 165 before this patch (the same
#    count over upstream main).  20 sites already cache through `LazyLock`
#    and four passes now read per-thread state; migrating the rest is a separate
#    patch with its own validation, but the count must only go down.  Re-measure
#    with the pipeline the check itself uses:
#
#      grep -rnI --include='*.rs' -e 'env::var(' -e 'env::var_os(' src/passes \
#        | grep -v '^src/passes/mod\.rs:' | wc -l
#
# 4. No deferred-work markers survive in src/, tests/ or scripts/.  A marker is a
#    claim that someone will come back; this tree's answer is to do the work, or
#    to write down why it is not needed.
set -euo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repo_root"

# Assembled so this file does not match its own check.
MARKER='FIX''ME'
# Budget for check 3.  Measured with the same pipeline the check uses; lower it
# whenever a pass is migrated, and never raise it.
ENV_READ_BUDGET=157

fail=0
report() { # report <label> <offending lines>
    if [[ -n $2 ]]; then
        printf 'FAIL %s:\n%s\n' "$1" "$2" >&2
        fail=1
    else
        printf 'ok   %s\n' "$1"
    fi
}

# 1. environment mutation is the guard module's business alone (call syntax, so
#    prose that discusses the hazard does not trip it)
hits=$(grep -rnI --include='*.rs' -e 'env::set_var(' -e 'env::remove_var(' src tests 2>/dev/null |
    grep -v '^src/test_support\.rs:' || true)
report "environment mutation confined to src/test_support.rs" "$hits"

# 2. the fully migrated passes stay free of environment reads
hits=$(grep -nI -e 'env::var(' -e 'env::var_os(' \
    src/passes/vec_interleave.rs src/passes/vec_load_sink.rs 2>/dev/null || true)
report "vec_interleave and vec_load_sink read no environment variable" "$hits"

# 3. the pipeline's environment reads do not grow
count=$(grep -rnI --include='*.rs' -e 'env::var(' -e 'env::var_os(' src/passes 2>/dev/null |
    grep -v '^src/passes/mod\.rs:' | wc -l | tr -d '[:space:]')
if (( count > ENV_READ_BUDGET )); then
    printf 'FAIL pass-pipeline environment reads grew: %s > budget %s\n' \
        "$count" "$ENV_READ_BUDGET" >&2
    printf '     newest offenders:\n' >&2
    grep -rnI --include='*.rs' -e 'env::var(' -e 'env::var_os(' src/passes 2>/dev/null |
        grep -v '^src/passes/mod\.rs:' | tail -5 >&2
    fail=1
else
    printf 'ok   pass-pipeline environment reads: %s (budget %s)\n' "$count" "$ENV_READ_BUDGET"
fi

# 4. no deferred-work markers
hits=$(grep -rnI -e "$MARKER" src tests scripts 2>/dev/null || true)
report "no deferred-work markers in src/, tests/ or scripts/" "$hits"

if (( fail != 0 )); then
    echo "check_env_test_hygiene: FAILED" >&2
    exit 1
fi
echo "check_env_test_hygiene: PASS (guard-only env mutation, migrated passes clean, reads not growing, no markers)"
