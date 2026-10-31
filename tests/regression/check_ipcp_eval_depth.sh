#!/usr/bin/env bash
# Exercise the evaluator with a positive fold and a deeper refusal on the same
# recursive shape. A small call that remains in main would make the refusal
# check vacuous; a deep call that disappears would defeat the stack-safety test.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
repo=$(cd "$(dirname "$0")/../.." && pwd)
tmp=${TMPDIR:-/tmp}/lccc-ipcpdepth.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
src="$repo/tests/regression/ipcp_eval_depth.c"

if ! "$CCC" -O2 -S "$src" -o "$tmp/depth.s" 2>"$tmp/asm.err"; then
  printf 'FAIL: assembly compilation failed: %s\n' "$(tail -3 "$tmp/asm.err")" >&2
  exit 1
fi
if ! "$CCC" -O2 -o "$tmp/depth" "$src" 2>"$tmp/link.err"; then
  printf 'FAIL: executable compilation failed: %s\n' "$(tail -3 "$tmp/link.err")" >&2
  exit 1
fi

# Restrict inspection to main so the recursive call in chain itself is not
# counted. Accept the target's call/callq spellings and an optional PLT suffix.
awk '/^main:/{in_main=1} in_main {print} /^\.size[[:space:]]+main,/{in_main=0}' \
  "$tmp/depth.s" > "$tmp/main.s"
calls=$(grep -Ec '^[[:space:]]*callq?[[:space:]]+chain(@PLT)?([[:space:]]|$)' "$tmp/main.s" || true)
if [ "$calls" -ne 1 ]; then
  printf 'FAIL: expected exactly one runtime call to chain in main (the deep call); found %s\n' "$calls" >&2
  cat "$tmp/main.s" >&2
  exit 1
fi

out=$("$tmp/depth")
if [ "$out" != "16996" ]; then
  printf 'FAIL: shallow constant recursion produced %s; expected 16996\n' "$out" >&2
  exit 1
fi
printf '%s\n' 'ipcp eval depth: shallow call folded; deep constant recursion safely remains a runtime call'
