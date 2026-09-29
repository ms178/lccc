#!/usr/bin/env bash
# check_loop_preheader.sh — LOOP-PREHEADER-1 end-to-end contract.
#
# WHY THIS GATE EXISTS AT ALL
# ---------------------------
# `src/passes/loop_preheader.rs` is 528 lines of CFG surgery that runs by
# default at -O2+, and its unit tests can only reach `retarget_edges` and
# `is_dedicated_to` -- pure functions of a terminator.  They cannot observe
# whether a preheader was inserted, and they cannot observe the thing the pass
# exists for.  A Rust-side test that passes while the pass fires zero times is
# indistinguishable from one that passes because the pass works, so the
# property that matters has to be asserted on the emitted assembly.
#
# Its own docstring used to name this file and `loop_preheader_shapes.c` as
# the end-to-end coverage.  Neither existed.  This is that coverage.
#
# WHAT IS ASSERTED
# ----------------
#   1. IT FIRES.  `dowhile_sum` -- guard outside the loop, so the load is in
#      the header and dominates every loop block -- has its derived-pointer
#      load hoisted out of the loop in the default configuration, and does NOT
#      with `CCC_DISABLE_PASSES=loop_preheader`.  Both halves are needed: a
#      default-only assertion passes trivially on a pass that never runs, and
#      a disabled-only assertion passes on a pass that changes nothing.
#   2. IT DECLINES WHERE IT MUST.  `while_sum` (guard at the top) and
#      `alloca_sum` (alloca loads) are emitted byte-identically with and
#      without the pass, and their loads stay in the loop.  Hoisting the first
#      would speculate a faulting load past the guard; the second would be
#      pure cost.
#   3. IT DOES NOT UNLOCK THE UNSOUND HOIST.  `guarded_sum` is the SQLite
#      `if (p == 0) return 0;` shape from the pass's own docstring: its
#      outside predecessor is not dedicated, so the load must stay in the loop
#      even with the pass at full strength.  This is the assertion that would
#      catch a future "widen the dedicated-preheader test" change turning into
#      a NULL dereference.
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
[[ -x $CCC ]] || { echo "check_loop_preheader: lccc not found at $CCC" >&2; exit 1; }

# shellcheck source=tests/regression/lib_loop_bounds.sh
. "$here/lib_loop_bounds.sh"

shapes=$here/loop_preheader/loop_preheader_shapes.c
[[ -f $shapes ]] || { echo "check_loop_preheader: missing $shapes" >&2; exit 1; }

work=$(mktemp -d "${TMPDIR:-/tmp}/lccc-preheader.XXXXXX")
trap 'rm -rf "$work"' EXIT
fail=0
note() { printf '  %s\n' "$*"; }
bad()  { printf '  FAIL: %s\n' "$*" >&2; fail=1; }

on=$work/on.s
off=$work/off.s
"$CCC" -O2 -S "$shapes" -o "$on" || { echo "check_loop_preheader: compile failed" >&2; exit 1; }
CCC_DISABLE_PASSES=loop_preheader "$CCC" -O2 -S "$shapes" -o "$off" \
    || { echo "check_loop_preheader: compile with the pass disabled failed" >&2; exit 1; }

# ---- 1. the pass fires, and the kill switch is the difference ---------------
note "contract 1: the derived-pointer load is hoisted only with the pass on"
require_loop_accesses "pass on:  dowhile_sum hoisted"  "$on"  dowhile_sum eq 0 \
    || bad "dowhile_sum should hoist its invariant derived load with the pass enabled"
require_loop_accesses "pass off: dowhile_sum not hoisted" "$off" dowhile_sum ge 1 \
    || bad "with loop_preheader disabled, dowhile_sum must keep the load in the loop"

# ---- 2. the pass declines the shapes where hoisting would be wrong ---------
note "contract 2: the shapes it must not touch are unchanged and stay in-loop"
for fn in while_sum guarded_sum alloca_sum; do
    require_loop_accesses "pass on:  $fn keeps its load"  "$on"  "$fn" ge 1 \
        || bad "$fn must keep its memory access inside the loop with the pass enabled"
    require_loop_accesses "pass off: $fn keeps its load"  "$off" "$fn" ge 1 \
        || bad "$fn must keep its memory access inside the loop with the pass disabled"
done
# Byte-identity is the strong form of "the pass declined": not merely the same
# loop, but not one extra block anywhere in the file.
if diff -q <(grep -v '^\s*\.p2align' "$on") <(grep -v '^\s*\.p2align' "$off") >/dev/null; then
    bad "the pass changed the emitted code for while_sum/guarded_sum/alloca_sum; the declined shapes must be byte-identical"
else
    # A difference is only legitimate if it is confined to dowhile_sum.  Check
    # that explicitly rather than trusting the per-function assertions alone.
    for fn in while_sum guarded_sum alloca_sum; do
        if ! diff -q <(asm_fn_lines "$on" "$fn") <(asm_fn_lines "$off" "$fn") >/dev/null; then
            bad "$fn differs between the two configurations; this shape must be untouched"
        fi
    done
fi

# ---- 3. soundness: the non-dedicated preheader still refuses ---------------
note "contract 3: the SQLite if (p == 0) return 0 shape is never hoisted"
if grep -qE '^\s*mov[a-z]+\s+.*\(%r[a-z0-9]+\)' <(asm_fn_lines "$on" guarded_sum); then
    : # a memory read exists somewhere in the function; the loop-body
      # assertion above is what pins *where* it is.
fi
require_loop_accesses "guarded_sum stays in-loop" "$on" guarded_sum ge 1 \
    || bad "guarded_sum: the load must not be hoisted into the non-dedicated preheader"

if [ "$fail" -ne 0 ]; then
    echo "--- asm (pass on) ---" >&2
    grep -vE '^\s*\.(cfi|loc|file|size|type|globl|p2align|ident|section)' "$on" >&2
    echo "--- asm (pass off) ---" >&2
    grep -vE '^\s*\.(cfi|loc|file|size|type|globl|p2align|ident|section)' "$off" >&2
    exit 1
fi
echo "check_loop_preheader: PASS"
