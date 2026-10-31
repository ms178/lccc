#!/usr/bin/env bash
# One-level direct self-recursion clone (`clone_self_recursion_sites`).
#
# The transform is invisible in a behavioural test — `binary_trees` prints the
# same checksum with and without it — so this gate reads the assembly and pins
# the three properties that make it correct *and* the three limits that keep it
# from becoming a fixed-point inliner:
#
#   fired:      a small non-tail body (`walk_tree`/`count_tree`/`free_tree`)
#               goes from N self calls to exactly 2N: every site is expanded
#               once, so the call/return of one recursion level disappears.
#   one level:  2N and not 4N — a second inliner invocation must not clone the
#               clone.
#   shaped:     `tail_sum` (tail self call, owned by `tail_call_elim`) and
#               `big_walk` (175 IR instructions, above
#               MAX_SELF_CLONE_INSTRUCTIONS) keep exactly their original call
#               count, so the tail-shape and size gates are not decoration.
#
# The negative control is the documented kill switch
# (`CCC_NO_SELF_RECURSION_CLONE=1`), which must restore the original call
# counts exactly; a gate whose "off" build looks like the "on" build is
# measuring nothing.
#
# Usage: CCC=target/fastbuild/lccc bash tests/regression/check_self_recursion_clone.sh
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
repo=$(cd "$(dirname "$0")/../.." && pwd)
tmp=${TMPDIR:-/tmp}/lccc-selfrec.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
fail=0

note() { printf '%s\n' "$*"; }
bad() {
  printf 'FAIL: %s\n' "$*"
  fail=1
}

# Body of one function, from its label to its `.size` directive.
body() { awk -v fn="$2" '$0 ~ ("^" fn ":") { on = 1 } on { print } $0 ~ ("^\\.size " fn ",") { on = 0 }' "$1"; }

# Direct self calls inside the function's own body (the callers of `main` are
# outside the body, so they do not count).
self_calls() {
  body "$1" "$2" | grep -cE "^[[:space:]]+call $2\$" || true
}

src="$repo/tests/regression/self_recursion_clone.c"
"$CCC" -O2 -S "$src" -o "$tmp/on.s"
CCC_NO_SELF_RECURSION_CLONE=1 "$CCC" -O2 -S "$src" -o "$tmp/off.s"

# ── 1. the clone fires, exactly one level ───────────────────────────────────
for fn in walk_tree count_tree free_tree; do
  on=$(self_calls "$tmp/on.s" "$fn")
  off=$(self_calls "$tmp/off.s" "$fn")
  if [ "$off" != "2" ]; then
    bad "$fn: the kill-switch build has $off self calls, expected 2 (fixture drifted)"
  fi
  if [ "$on" != "4" ]; then
    bad "$fn: $on self calls after the clone, expected 4 (2 sites expanded once)"
  fi
done

# ── 2. the limits hold ──────────────────────────────────────────────────────
# Tail position is `tail_call_elim`'s shape, and an oversized body is outside
# MAX_SELF_CLONE_INSTRUCTIONS: both must keep the same call count AND the same
# instruction count with and without the cloner. (The bodies are not
# byte-identical: a clone in an earlier function shifts every later `.LBB`
# label, which is a numbering change, not a shape change.)
insn_count() { body "$1" "$2" | grep -cE '^[[:space:]]+[a-z]' || true; }
for fn in tail_sum big_walk; do
  if [ -z "$(body "$tmp/on.s" "$fn")" ]; then
    bad "$fn: not present in the assembly (fixture drifted)"
    continue
  fi
  on_calls=$(self_calls "$tmp/on.s" "$fn")
  off_calls=$(self_calls "$tmp/off.s" "$fn")
  on_insns=$(insn_count "$tmp/on.s" "$fn")
  off_insns=$(insn_count "$tmp/off.s" "$fn")
  if [ "$on_calls" != "$off_calls" ] || [ "$on_insns" != "$off_insns" ]; then
    bad "$fn: the cloner changed a body it must leave alone (calls $off_calls -> $on_calls, instructions $off_insns -> $on_insns)"
  fi
done
# ...and `big_walk` really is above the size gate while keeping its one call.
big_calls=$(self_calls "$tmp/off.s" big_walk)
if [ "$big_calls" != "1" ]; then
  bad "big_walk: $big_calls self calls, expected the 1 outlined call"
fi

# ── 3. semantics: cloned and uncloned builds agree ──────────────────────────
"$CCC" -O2 -o "$tmp/on" "$src"
CCC_NO_SELF_RECURSION_CLONE=1 "$CCC" -O2 -o "$tmp/off" "$src"
"$tmp/on" >"$tmp/on.out"
"$tmp/off" >"$tmp/off.out"
if ! cmp -s "$tmp/on.out" "$tmp/off.out"; then
  bad "clone-on and clone-off builds disagree: $(cat "$tmp/on.out") vs $(cat "$tmp/off.out")"
fi
if ! grep -qE '^[0-9]+$' <<<"$(cat "$tmp/on.out")"; then
  bad "unexpected output shape: $(cat "$tmp/on.out")"
fi

if [ "$fail" = "0" ]; then
  note "self-recursion clone: one level, tail calls untouched, oversized bodies refused"
fi
exit "$fail"
