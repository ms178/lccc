#!/usr/bin/env bash
# Source-only helper: compare successful, bounded executions, never substitutions
# whose exit statuses are discarded by a surrounding `[` conditional.
tls_compare_runtime() {
    local candidate=$1 reference=$2 output_prefix=$3 limit=$4
    shift 4
    local role program status
    for role in reference candidate; do
        if [[ $role == reference ]]; then
            program=$reference
        else
            program=$candidate
        fi
        if timeout "$limit" "$program" "$@" >"$output_prefix.$role"; then
            :
        else
            status=$?
            if [[ $status -eq 124 ]]; then
                printf 'TLS %s timed out\n' "$role" >&2
            else
                printf 'TLS %s failed (status %s)\n' "$role" "$status" >&2
            fi
            return 1
        fi
    done
    if ! cmp -s "$output_prefix.reference" "$output_prefix.candidate"; then
        printf '%s\n' 'TLS candidate output differs from reference' >&2
        return 1
    fi
}
