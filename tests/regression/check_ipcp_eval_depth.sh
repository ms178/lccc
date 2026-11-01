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

fail() {
  local message=$1
  printf 'FAIL: %s\n' "$message" >&2
  # Publish the actionable failure in the GitHub check annotation as well as
  # the step log, so red-team failures remain diagnosable from the check API.
  message=${message//'%'/'%25'}
  message=${message//$'\r'/'%0D'}
  message=${message//$'\n'/'%0A'}
  printf '::error title=IPCP evaluation-depth regression::%s\n' "$message"
  exit 1
}

if ! "$CCC" -O2 -S "$src" -o "$tmp/depth.s" 2>"$tmp/asm.err"; then
  fail "assembly compilation failed: $(tail -3 "$tmp/asm.err")"
fi
if ! "$CCC" -O2 -o "$tmp/depth" "$src" 2>"$tmp/link.err"; then
  fail "executable compilation failed: $(tail -3 "$tmp/link.err")"
fi

# Restrict inspection to main so the recursive call in chain itself is not
# counted. Accept the target's call/callq spellings and an optional PLT suffix.
awk '/^main:/{in_main=1} in_main {print} /^\.size[[:space:]]+main,/{in_main=0}' \
  "$tmp/depth.s" > "$tmp/main.s"
calls=$(grep -Ec '^[[:space:]]*callq?[[:space:]]+chain(@PLT)?([[:space:]]|$)' "$tmp/main.s" || true)
if [ "$calls" -ne 1 ]; then
  cat "$tmp/main.s" >&2
  fail "expected exactly one runtime call to chain in main (the deep call); found $calls"
fi

out=$("$tmp/depth")
if [ "$out" != "16996" ]; then
  fail "shallow constant recursion produced $out; expected 16996"
fi
printf '%s\n' 'ipcp eval depth: shallow call folded; deep constant recursion safely remains a runtime call'
