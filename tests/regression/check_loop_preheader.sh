#!/usr/bin/env bash
# LOOP-PREHEADER-1, end to end: does the dedicated-preheader pass actually
# unlock the hoist it exists for -- and does it do so without ever moving a
# dereference onto a path that must not execute it?
#
# `loop_preheader.rs` claims in its module doc that the transformation is
# covered by this script. It was not: neither file existed, so a 528-line
# CFG-surgery pass enabled by default at -O2+ was guarded only by three unit
# tests of its pure helpers. This is the missing half.
#
#   1. RUNTIME      the shapes compute the right answers with the pass on and
#                   off. A hoist that fires when it must not would fault.
#   2. EFFECT       `guarded_sum` (the SQLite shape) loads p->nUsed ONCE,
#                   OUTSIDE the loop, when the pass runs -- and reloads it
#                   every iteration when the pass is disabled. That delta is
#                   the entire point of the pass. `dowhile_sum` (2b) is the
#                   same contract on a do/while, which is entered by FALLING
#                   IN rather than by a back edge: a different lowering, and
#                   the case that would catch a preheader wrongly claimed on
#                   the fall-in edge. Both carry their own negative control.
#   3. SOUNDNESS    the hoisted load sits AFTER the `p == 0` early return. If
#                   it were hoisted into the guard block instead of a
#                   dedicated preheader, the NULL path would dereference it.
#                   This is the miscompile the pass was written to prevent.
#   4. SCOPE        seven shapes the pass must NOT touch are each left
#                   byte-identical between the two arms:
#                     `invariant_ptr`     no guard, nothing to unlock;
#                     `switch_entered`    two outside edges -- no single block
#                                         dominates the header, so claiming a
#                                         preheader would leave the hoisted
#                                         bound undefined on the other path;
#                     `computed_goto`     IndirectBranch into the header --
#                                         there is no edge to reroute;
#                     `already_dedicated` the while-form already has one;
#                     `while_sum`         guard-at-top, so the header is
#                                         itself a loop block and is not
#                                         must-execute;
#                     `null_guard_sum`    the guard block is the unique
#                                         outside predecessor but also
#                                         branches to the early return, so
#                                         it is not DEDICATED -- hoisting
#                                         `p[0]` into it would deref NULL;
#                     `alloca_sum`        every load reads an alloca, so a
#                                         preheader unlocks nothing and the
#                                         inserted block is pure cost.
#                   `switch_entered` and `computed_goto` are the miscompile
#                   shapes: counting only Branch edges would report one of
#                   two entering blocks as *the* preheader, and the def of
#                   the hoisted bound would then not dominate its use.
#                   `null_guard_sum` is the soundness shape the pass's own
#                   docstring cites; `alloca_sum` is the profitability shape.
#   5. CONTROL      contract 2's disabled-pass arm is the negative control.
#                   Without it, a pass that never fired would also show a
#                   single load outside the loop and pass vacuously.
#   6. IDEMPOTENCE  with CCC_DEBUG_LOOP_PREHEADER set, the fixpoint's second
#                   pass over the same header reports "already has a
#                   dedicated preheader" and inserts nothing further. A pass
#                   that kept inserting would either loop forever or emit a
#                   chain of empty blocks.
#   7. CENSUS      every merged loop the final round settled on lands in
#                   exactly one bucket, so `loops` equals the sum of its
#                   reasons. The pass asserts this internally, but
#                   `fastbuild` inherits `release`, where `debug-assertions`
#                   is off, so that assert is compiled out of a CI binary;
#                   checking the REPORTED numbers here is what makes the
#                   invariant profile-independent and end-to-end.
set -uo pipefail
cd "$(dirname "$0")/../.."

ccc=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
src=tests/regression/loop_preheader_shapes.c
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

[[ -x $ccc ]] || { echo "check_loop_preheader: lccc not found at $ccc" >&2; exit 1; }

march=$("$ccc" -march=x86-64-v3 -E -x c /dev/null -o /dev/null 2>/dev/null \
    && echo "-march=x86-64-v3" || echo "")

rc=0
fail() { echo "FAIL: $*" >&2; rc=1; }

# ── 1. runtime, with and without the pass ────────────────────────────────
for mode in on off; do
    if [ "$mode" = on ]; then
        unset CCC_DISABLE_PASSES
    else
        export CCC_DISABLE_PASSES=loop_preheader
    fi
    if ! "$ccc" -O2 $march "$src" -o "$td/rt-$mode" 2>"$td/cc-$mode.log"; then
        fail "compile failed with the pass $mode"
        sed -n '1,10p' "$td/cc-$mode.log" >&2
        continue
    fi
    if ! "$td/rt-$mode"; then
        fail "wrong answer with the pass $mode (exit $?)"
    fi
done
unset CCC_DISABLE_PASSES
[ $rc -eq 0 ] && echo "ok: runtime correct with the pass on and off"

# ── assembly for both arms ──────────────────────────────────────────────
unset CCC_DISABLE_PASSES
"$ccc" -O2 $march -S "$src" -o "$td/on.s" || { fail "cannot emit asm (on)"; exit 1; }
CCC_DISABLE_PASSES=loop_preheader "$ccc" -O2 $march -S "$src" -o "$td/off.s" \
    || { fail "cannot emit asm (off)"; exit 1; }
unset CCC_DISABLE_PASSES

# A memory operand using ONLY the base register: `(%rdi)` is the nUsed load.
# The `a[]` accesses are indexed (`(%rsi,%r9,4)`) and never match this.
base_only='\([[:space:]]*%r[a-z0-9]+\)'

fn_body() { awk -v f="$1" '$0==f":"{ins=1} ins{print} /^\.size/{if(ins)exit}' "$2"; }

# Slice the loop: from the backward branch's target label to that branch.
loop_region() {
    awk -v f="$1" '$0==f":"{ins=1} ins{print} /^\.size/{if(ins)exit}' "$2" | awk '
        { lines[NR]=$0 }
        /^[[:space:]]*j[a-z]+[[:space:]]+\.[A-Za-z]/ { last=NR }
        END {
            if (!last) exit 1
            lbl=lines[last]; sub(/^[[:space:]]*j[a-z]+[[:space:]]+/,"",lbl); gsub(/[[:space:]]/,"",lbl)
            for (i=1;i<=NR;i++) {
                l=lines[i]; sub(/:$/,"",l); gsub(/[[:space:]]/,"",l)
                if (l==lbl) { start=i; break }
            }
            if (!start) exit 1
            body=""
            for (i=start;i<=last;i++) body=body lines[i] "\n"
            print body
        }'
}

# ── 2. effect: the hoist fires, and only when the pass runs ─────────────
on_loop=$(loop_region guarded_sum "$td/on.s")
off_loop=$(loop_region guarded_sum "$td/off.s")

on_in=$(echo "$on_loop"  | grep -Ec "$base_only")
off_in=$(echo "$off_loop" | grep -Ec "$base_only")

if [ "$on_in" -eq 0 ]; then
    echo "ok: guarded_sum hoists p->nUsed out of the loop"
else
    fail "guarded_sum still loads p->nUsed inside the loop ($on_in access(es))"
fi
if [ "$off_in" -ge 1 ]; then
    echo "ok: negative control -- with the pass off it reloads every iteration"
else
    fail "negative control failed: the pass-off build also has no in-loop load, \
so contract 2 proves nothing"
fi

# ── 3. soundness: the load must not precede the NULL early return ───────
on_body=$(fn_body guarded_sum "$td/on.s")
ret_ln=$(echo "$on_body" | grep -nE '^[[:space:]]*ret' | head -1 | cut -d: -f1)
ld_ln=$(echo "$on_body"  | grep -nE "$base_only"       | head -1 | cut -d: -f1)
if [ -n "$ret_ln" ] && [ -n "$ld_ln" ] && [ "$ret_ln" -lt "$ld_ln" ]; then
    echo "ok: the hoisted load sits after the p==0 early return (line $ld_ln > $ret_ln)"
elif [ -z "$ld_ln" ]; then
    fail "no base-register load found in guarded_sum at all"
else
    fail "the hoisted load precedes the p==0 return (load@$ld_ln, ret@$ret_ln): \
the NULL path would dereference"
fi

# ── 2b. effect, second positive shape: the do/while lowering ────────────
# `guarded_sum` proves the hoist fires for a `for` loop entered from a
# guard. `dowhile_sum` is the same rule on a do/while, where the loop is
# entered by FALLING IN rather than by a back edge -- a different lowering,
# and the one case that would catch a preheader inserted on the fall-in
# edge. Same contract, so the same measurement.
dw_on_loop=$(loop_region dowhile_sum "$td/on.s")
dw_off_loop=$(loop_region dowhile_sum "$td/off.s")
dw_on_in=$(echo "$dw_on_loop"  | grep -Ec "$base_only")
dw_off_in=$(echo "$dw_off_loop" | grep -Ec "$base_only")

if [ "$dw_on_in" -eq 0 ]; then
    echo "ok: dowhile_sum hoists c[0] out of the do/while body"
else
    fail "dowhile_sum still loads c[0] inside the loop ($dw_on_in access(es))"
fi
if [ "$dw_off_in" -ge 1 ]; then
    echo "ok: negative control -- do/while reloads every iteration with the pass off"
else
    fail "negative control failed for dowhile_sum: the pass-off build also has \
no in-loop load, so that half of the contract proves nothing"
fi

# ── 4. scope: every shape the pass must refuse is left untouched ───────
for fn in invariant_ptr switch_entered computed_goto already_dedicated \
         while_sum null_guard_sum alloca_sum; do
    if diff <(fn_body "$fn" "$td/on.s") <(fn_body "$fn" "$td/off.s") >/dev/null; then
        echo "ok: $fn is byte-identical (pass correctly declined)"
    else
        fail "$fn changed between the two arms: the pass inserted where it \
must not"
        diff <(fn_body "$fn" "$td/on.s") <(fn_body "$fn" "$td/off.s") | head -8 >&2
    fi
done

# ── 5. idempotence: the fixpoint stops after one insertion ─────────────
trace=$(CCC_DEBUG_LOOP_PREHEADER=1 "$ccc" -O2 $march -S "$src" -o /dev/null 2>&1)
inserts=$(echo "$trace" | grep -c "is not dedicated: inserting")
settled=$(echo "$trace" | grep -c "already has a dedicated preheader")
if [ "$inserts" -ge 1 ] && [ "$settled" -ge 1 ]; then
    echo "ok: idempotent -- $inserts insertion(s), then the fixpoint settles \
($settled 'already dedicated')"
else
    fail "idempotence broken: inserts=$inserts settled=$settled (want >=1 each). \
Trace was:\n$trace"
fi

# ── 6. the census is the sum of its parts ───────────────────────────────
# The pass asserts this internally, but `fastbuild` inherits `release`, where
# `debug-assertions` is off, so that assert would be compiled out of a CI
# binary. Checking the REPORTED numbers here makes the invariant
# profile-independent and end-to-end: every merged loop the final round
# settled on must appear in exactly one bucket.
trace=$(CCC_DEBUG_LOOP_PREHEADER=1 "$ccc" -O2 $march -S "$src" -o /dev/null 2>&1)
census_lines=$(printf '%s\n' "$trace" | grep -c "LOOP-PREHEADER-CENSUS")
if [ "$census_lines" -eq 0 ]; then
    fail "no census was reported; contract 6 cannot be checked and the \
census is not observable, so 'the pass never fires' is indistinguishable \
from 'the pass is blind'"
else
    unbalanced=$(printf '%s\n' "$trace" | grep "LOOP-PREHEADER-CENSUS" | awk '
        { b = 0
          for (i = 1; i <= NF; i++) {
              split($i, kv, "=")
              if      (kv[1] == "loops")                   loops = kv[2] + 0
              else if (kv[1] == "profitability")           b += kv[2] + 0
              else if (kv[1] == "no_single_outside_pred")  b += kv[2] + 0
              else if (kv[1] == "already_dedicated")       b += kv[2] + 0
              else if (kv[1] == "indirect_pred")           b += kv[2] + 0
              else if (kv[1] == "out_of_range")            b += kv[2] + 0
          }
          if (loops != b) n++ }
        END { print n + 0 }')
    if [ "$unbalanced" -eq 0 ]; then
        echo "ok: census is balanced across $census_lines report(s) -- every \
loop is in exactly one bucket"
    else
        fail "$unbalanced census report(s) where loops != sum(buckets): the \
rejection buckets do not partition the loops, so the headline number and \
its reasons have drifted apart"
    fi
fi

exit $rc
