#!/usr/bin/env bash
# Sourced by run.sh. Keep the speedtest database check testable with synthetic
# SQLite CLIs: a successful `[[ $(sqlite3 ...) == ok ]]` can discard the CLI's
# nonzero exit status, and stdout alone cannot reveal a runtime warning.
sqlite_speedtest_integrity() {
    local exe=$1 db=$2 out=$3 err=$4 status=0
    timeout --kill-after=5s 120 "$exe" -batch -bail "$db" \
        'PRAGMA integrity_check;' > "$out" 2> "$err" || status=$?
    if (( status != 0 )) || [[ $(cat "$out") != ok || -s "$err" ]]; then
        printf 'speedtest DB failed integrity_check (exit %s; stdout %s; stderr %s): %s\n' \
            "$status" "$out" "$err" "$db" >&2
        return 1
    fi
    return 0
}
