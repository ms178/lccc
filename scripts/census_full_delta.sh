#!/usr/bin/env bash
# census_full_delta.sh — per-TU static A/B across ALL optimization levels.
#
# For every C TU under the benchmark + kernel + patterns + regression corpora,
# compile with NEW and OLD lccc at -O0/-O1/-O2/-O3/-Os and report any TU whose
# instruction or frame-slot-reference count differs:
#
#   -$opt insn:<d_insn> stk:<d_stk> <path>      (d = NEW - OLD)
#
# Positive insn/stk deltas are regressions and must be triaged; negative are
# improvements. This is the whole-corpus static screen that complements
# census_ab.py (which gates on fixed buckets at one opt level).
#
# Usage: census_full_delta.sh OLD_LCCC NEW_LCCC [jobs]
set -uo pipefail

OLD=${1:?usage: $0 OLD_LCCC NEW_LCCC [jobs]}
NEW=${2:?usage: $0 OLD_LCCC NEW_LCCC [jobs]}
JOBS=${3:-$(nproc 2>/dev/null || echo 2)}
REPO=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$REPO"

DIRS=(tests/benchmark/programs tests/benchmark/kernel_corpus
      tests/benchmark/patterns tests/regression)

# Stats per emitted .s: "insns stkrefs"
asm_stats() {
    awk '
    /^[[:space:]]*\./ { next }
    /^[[:space:]]*#/  { next }
    /:$/ && /^[^[:space:]]/ { next }
    /^[[:space:]]*[a-zA-Z]/ {
        ins++
        if ($0 ~ /-?[0-9]+\(%r(sp|bp)/) stk++
    }
    END { printf "%d %d\n", ins+0, stk+0 }
    ' "$1"
}

work=$(mktemp -d /tmp/census_fd.XXXXXX)
trap 'rm -rf "$work"' EXIT
: > "$work/tasks"
for d in "${DIRS[@]}"; do
    for f in "$d"/*.c; do
        [ -f "$f" ] || continue
        echo "$f"
    done
done > "$work/files"

pid=0
while IFS= read -r f; do
    (
        wd=$(mktemp -d "$work/w.XXXXXX")
        rel=${f#"$REPO/"}
        for opt in O0 O1 O2 O3 Os; do
            if ! "$NEW" -"$opt" -S -o "$wd/n.s" "$f" >/dev/null 2>&1; then continue; fi
            if ! "$OLD" -"$opt" -S -o "$wd/o.s" "$f" >/dev/null 2>&1; then continue; fi
            read -r ni ns < <(asm_stats "$wd/n.s")
            read -r oi os < <(asm_stats "$wd/o.s")
            di=$((ni-oi)); ds=$((ns-os))
            if [ "$di" -ne 0 ] || [ "$ds" -ne 0 ]; then
                echo "-$opt insn:$di stk:$ds $rel"
            fi
        done
    ) &
    pid=$((pid+1))
    if [ $((pid % JOBS)) -eq 0 ]; then wait; fi
done < "$work/files"
wait
