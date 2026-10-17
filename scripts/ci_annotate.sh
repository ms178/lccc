#!/usr/bin/env bash
# Run one CI gate and surface its failing lines as API-visible annotations.
#
# WHY: the regression gates print precise `FAIL <case>` / `mismatch ...` lines,
# but those go to the job log, and a job log is fetched from blob storage that
# is not always reachable (the checks UI then shows only "exit code 1"). The
# formatting gate already solves this by emitting `::error::` annotations; this
# wrapper gives every gate the same property without changing the gate.
#
# Usage: bash scripts/ci_annotate.sh <label> -- <command...>
# Exit status is the wrapped command's status.
set -uo pipefail

label=${1:?usage: ci_annotate.sh <label> -- <command...>}
shift
if [[ ${1:-} == -- ]]; then
    shift
fi

tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT

"$@" 2>&1 | tee "$tmp"
status=${PIPESTATUS[0]}

if (( status != 0 )); then
    emitted=0
    while IFS= read -r line; do
        case $line in
            *FAIL* | *mismatch* | *error* | *Error*) ;;
            *) continue ;;
        esac
        escaped=${line//'%'/'%25'}
        escaped=${escaped//$'\r'/'%0D'}
        printf '::error title=%s::%s\n' "$label" "$escaped"
        emitted=$((emitted + 1))
        (( emitted >= 25 )) && break
    done <"$tmp"
    if (( emitted == 0 )); then
        # No recognisable failure line: attach the tail so the annotation is
        # still actionable rather than a bare exit code.
        tail -n 25 "$tmp" | while IFS= read -r line; do
            escaped=${line//'%'/'%25'}
            escaped=${escaped//$'\r'/'%0D'}
            printf '::error title=%s::%s\n' "$label" "$escaped"
        done
    fi
fi

exit "$status"
