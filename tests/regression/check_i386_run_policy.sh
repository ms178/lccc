#!/usr/bin/env bash
# Deterministic policy tests. Memo injection is confined to this test; the
# production probe never trusts a user-supplied capability or tests LCCC.
set -euo pipefail
source "$(dirname "$0")/i386_exec.sh"
for capability in run link none; do
    _I386_CAP_MEMO[host-probe-stub]=$capability
    for required in 0 1; do
        rc=0
        output=$(LCCC_REQUIRE_I686=$required i386_run_or_skip host-probe-stub 2>&1) || rc=$?
        if [[ $capability == run ]]; then
            [[ $rc == 0 && -z $output ]]
        elif [[ $required == 1 ]]; then
            [[ $rc == 1 && $output == FAIL:* && $output != *SKIP* ]]
        else
            [[ $rc == 77 && $output == SKIP-RUN:* ]]
        fi
    done
done
echo 'PASS: i686 run/link/none × optional/required policy'
