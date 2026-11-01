#!/usr/bin/env bash
# One-level direct self-recursion clone (`clone_self_recursion_sites`).
#
# The transform is invisible in a behavioural test — `binary_trees` prints the
# same checksum with and without it — so this gate reads the assembly and pins
# the properties that make it correct *and* the limits that keep it from
# becoming a fixed-point inliner:
#
#   fired:      a small non-tail body (`walk_tree`/`count_tree`/`free_tree`)
#               goes from N self calls to exactly 2N: every site is expanded
#               once, so the call/return of one recursion level disappears.
#   one level:  2N and not 4N — a second inliner invocation must not clone the
#               clone.
#   shaped:     `tail_sum` (tail self call, owned by `tail_call_elim`) and
#               `big_walk` (175 IR instructions, above
#               MAX_SELF_CLONE_INSTRUCTIONS) keep exactly their original call
#               and instruction counts, so the tail-shape and size gates are
#               not decoration.
#   partition:  at -O2 and above the clone is deferred to the post-structural
#               inliner and skips every function `self_inline` already
#               expanded. Cloning first (as the clone originally ran, inside the
#               primary inliner) hands `rec2iter`/`tce_acc` a body with twice
#               the self calls, which they refuse, and leaves `self_inline`
#               nothing to do: binary_trees -O2 measured +4.0 % Ir / +4.1 %
#               mispredicted branches on x86-64 and +7.1 % Ir / +32.6 %
#               mispredicts on i686 against the partitioned order. So the
#               default -O2 build of the fixture must equal the kill-switch
#               build exactly.
#
# Where each property is visible:
#   -O1 has no `self_inline`, so the clone is the only expander: "fired",
#   "one level" and "shaped" are pinned on the default build there.
#   -O2 pins "partition" on the default build, and pins "fired"/"one level"
#   again with `self_inline` disabled (CCC_DISABLE_PASSES=self_inline), where
#   the deferred clone is what runs. `count_tree` keeps one call at -O2 (the
#   accumulator rewrite turns one of its two self calls into a loop), so there
#   the clone's one level shows as a larger body around that one call instead.
#
# The negative control is the documented kill switch
# (`CCC_NO_SELF_RECURSION_CLONE=1`), which must restore the original shapes
# exactly; a gate whose "off" build looks like the "on" build is measuring
# nothing.
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
insn_count() { body "$1" "$2" | grep -cE '^[[:space:]]+[a-z]' || true; }

# build NAME OPT [ENV...]: assemble the fixture into "$tmp/NAME.s".
build() {
  local name=$1 opt=$2
  shift 2
  env "$@" "$CCC" "$opt" -S "$src" -o "$tmp/$name.s"
}
build o1_on -O1
build o1_off -O1 CCC_NO_SELF_RECURSION_CLONE=1
build o2_on -O2
build o2_off -O2 CCC_NO_SELF_RECURSION_CLONE=1
build o2_clone_only -O2 CCC_DISABLE_PASSES=self_inline
build o2_neither -O2 CCC_DISABLE_PASSES=self_inline CCC_NO_SELF_RECURSION_CLONE=1

# expect_calls FILE FN WANT WHAT
expect_calls() {
  local got
  got=$(self_calls "$tmp/$1.s" "$2")
  if [ "$got" != "$3" ]; then
    bad "$2 ($1): $got self calls, expected $3 ($4)"
  fi
}

# ── 1. the clone fires, exactly one level ───────────────────────────────────
# -O1: the clone is the only self-recursion expander.
for fn in walk_tree count_tree free_tree; do
  expect_calls o1_off "$fn" 2 "kill-switch build; fixture drifted"
  expect_calls o1_on "$fn" 4 "2 sites expanded once"
done
# -O2 with `self_inline` disabled: the deferred clone runs.
for fn in walk_tree free_tree; do
  expect_calls o2_neither "$fn" 2 "kill-switch build; fixture drifted"
  expect_calls o2_clone_only "$fn" 4 "2 sites expanded once"
done
expect_calls o2_neither count_tree 1 "accumulator rewrite leaves one self call; fixture drifted"
expect_calls o2_clone_only count_tree 1 "the one remaining site expanded once, not twice"
ct_off=$(insn_count "$tmp/o2_neither.s" count_tree)
ct_on=$(insn_count "$tmp/o2_clone_only.s" count_tree)
# One expanded level roughly doubles the body; a second level would quadruple
# it. Bound both sides so neither "did nothing" nor "cloned the clone" passes.
if [ "$ct_on" -le "$ct_off" ] || [ "$ct_on" -ge $((3 * ct_off)) ]; then
  bad "count_tree (o2_clone_only): body $ct_off -> $ct_on instructions, expected one expanded level (more than $ct_off, fewer than $((3 * ct_off)))"
fi

# ── 2. the limits hold ──────────────────────────────────────────────────────
# Tail position is `tail_call_elim`'s shape, and an oversized body is outside
# MAX_SELF_CLONE_INSTRUCTIONS: both must keep the same call count AND the same
# instruction count with and without the cloner. (The bodies are not
# byte-identical: a clone in an earlier function shifts every later `.LBB`
# label, which is a numbering change, not a shape change.)
same_shape() { # ON OFF FN
  if [ "$(self_calls "$tmp/$1.s" "$3")" != "$(self_calls "$tmp/$2.s" "$3")" ] ||
    [ "$(insn_count "$tmp/$1.s" "$3")" != "$(insn_count "$tmp/$2.s" "$3")" ]; then
    return 1
  fi
}
for fn in tail_sum big_walk; do
  for pair in "o1_on o1_off" "o2_clone_only o2_neither"; do
    set -- $pair
    if [ -z "$(body "$tmp/$1.s" "$fn")" ]; then
      bad "$fn: not present in $1 (fixture drifted)"
    elif ! same_shape "$1" "$2" "$fn"; then
      bad "$fn ($1): the cloner changed a body it must leave alone (calls $(self_calls "$tmp/$2.s" "$fn") -> $(self_calls "$tmp/$1.s" "$fn"), instructions $(insn_count "$tmp/$2.s" "$fn") -> $(insn_count "$tmp/$1.s" "$fn"))"
    fi
  done
done
# ...and `big_walk` really is above the size gate while keeping its one call.
expect_calls o1_off big_walk 1 "the 1 outlined call"

# ── 3. the partition: `self_inline` owns what it expands ────────────────────
for fn in walk_tree count_tree free_tree; do
  if ! same_shape o2_on o2_off "$fn"; then
    bad "$fn (-O2): the default build differs from the kill-switch build (calls $(self_calls "$tmp/o2_off.s" "$fn") -> $(self_calls "$tmp/o2_on.s" "$fn"), instructions $(insn_count "$tmp/o2_off.s" "$fn") -> $(insn_count "$tmp/o2_on.s" "$fn")): the clone ran on a function self_inline expanded"
  fi
  # ...and the equality is not vacuous: self_inline did expand it.
  if same_shape o2_on o2_clone_only "$fn"; then
    bad "$fn (-O2): the default build equals the self_inline-disabled build, so the partition check above proves nothing"
  fi
done

# ── 4. semantics: every configuration agrees ────────────────────────────────
ref=""
for cfg in "-O1" "-O1 CCC_NO_SELF_RECURSION_CLONE=1" "-O2" "-O2 CCC_NO_SELF_RECURSION_CLONE=1" \
  "-O2 CCC_DISABLE_PASSES=self_inline"; do
  set -- $cfg
  opt=$1
  shift
  env "$@" "$CCC" "$opt" -o "$tmp/prog" "$src"
  out=$("$tmp/prog")
  if ! grep -qE '^[0-9]+$' <<<"$out"; then
    bad "unexpected output shape ($cfg): $out"
  fi
  if [ -z "$ref" ]; then
    ref=$out
  elif [ "$out" != "$ref" ]; then
    bad "builds disagree: $ref (-O1) vs $out ($cfg)"
  fi
done

if [ "$fail" = "0" ]; then
  note "self-recursion clone: one level, tail calls untouched, oversized bodies refused, self_inline-expanded functions skipped"
fi
exit "$fail"
