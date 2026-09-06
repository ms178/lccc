#!/usr/bin/env bash
# PF-15's strcmp-shaped chain contains a byte->int cast used only for the
# loop's truth test and two same-width casts separated by the other byte load.
# Simplification must expose the original byte values before register allocation:
# `testb` and `cmpb` prove both extensions disappeared without relying on a
# deferred move surviving an intervening load.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
src="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/widened_byte_chain.c"
"$CCC" -O2 -S "$src" -o "$td/chain.s"

function_body() {
    awk '
        $0 == "strcmp_byte_chain:" { active=1 }
        active { print }
        active && $0 == ".size strcmp_byte_chain, .-strcmp_byte_chain" { exit }
    ' "$1"
}

body=$(function_body "$td/chain.s")
if ! grep -Eq '^[[:space:]]+testb[[:space:]]+%[a-z0-9]+, %[a-z0-9]+' <<<"$body"; then
    echo "FAIL: widened byte condition did not fold to testb" >&2
    printf '%s\n' "$body" >&2
    exit 1
fi
if ! grep -Eq '^[[:space:]]+cmpb[[:space:]]+%[a-z0-9]+, %[a-z0-9]+' <<<"$body"; then
    echo "FAIL: non-adjacent widened byte pair did not fold to cmpb" >&2
    printf '%s\n' "$body" >&2
    exit 1
fi
# A register-source movsbq in this function is the old widening-cast relay
# (`movsbq %dl,%rX` / `movsbq %al,%rX`).  Memory-source movsbq remains allowed:
# the return expression converts the final byte to unsigned int separately.
if grep -Eq '^[[:space:]]+movsbq[[:space:]]+%[a-z0-9]+, %[a-z0-9]+' <<<"$body"; then
    echo "FAIL: byte cast relay survived the narrow condition/pair fold" >&2
    printf '%s\n' "$body" >&2
    exit 1
fi

echo "OK widened_byte_chain"
