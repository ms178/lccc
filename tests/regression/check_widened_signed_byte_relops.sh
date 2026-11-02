#!/usr/bin/env bash
# Structural PF-15 companion for widened_signed_byte_relops.c.  The runtime
# regression exhausts signed inputs; this makes every signed predicate's x86
# narrowing visible so a future refactor cannot silently leave one expensive
# widened compare behind.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
src="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/widened_signed_byte_relops.c"
"$CCC" -O2 -S "$src" -o "$td/relops.s"

function_body() {
    local fn=$1
    awk -v fn="$fn:" '
        $0 == fn { active=1 }
        active { print }
        active && $0 == ".size " substr(fn, 1, length(fn)-1) ", .-" substr(fn, 1, length(fn)-1) { exit }
    ' "$td/relops.s"
}

for spec in 'rel_eq sete' 'rel_ne setne' 'rel_lt setl' \
            'rel_le setle' 'rel_gt setg' 'rel_ge setge'; do
    read -r fn setcc <<<"$spec"
    body=$(function_body "$fn")
    if ! grep -Eq '^[[:space:]]+cmpb[[:space:]]+%[a-z0-9]+, %[a-z0-9]+' <<<"$body"; then
        echo "FAIL: $fn did not narrow to cmpb" >&2
        printf '%s\n' "$body" >&2
        exit 1
    fi
    if ! grep -Eq "^[[:space:]]+${setcc}[[:space:]]" <<<"$body"; then
        echo "FAIL: $fn did not retain signed condition code $setcc" >&2
        printf '%s\n' "$body" >&2
        exit 1
    fi
done

echo "OK widened_signed_byte_relops"
