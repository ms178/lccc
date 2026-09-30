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
#                   the entire point of the pass.
#   3. SOUNDNESS    the hoisted load sits AFTER the `p == 0` early return. If
#                   it were hoisted into the guard block instead of a
#                   dedicated preheader, the NULL path would dereference it.
#                   This is the miscompile the pass was written to prevent.
#   4. SCOPE        four shapes the pass must NOT touch are each left
#                   byte-identical between the two arms:
#                     `invariant_ptr`     no guard, nothing to unlock;
#                     `switch_entered`    two outside edges -- no single block
#                                         dominates the header, so claiming a
#                                         preheader would leave the hoisted
#                                         bound undefined on the other path;
#                     `computed_goto`     IndirectBranch into the header --
#                                         there is no edge to reroute;
#                     `already_dedicated` the while-form already has one.
#                   `switch_entered` and `computed_goto` are the miscompile
#                   shapes: counting only Branch edges would report one of
#                   two entering blocks as *the* preheader, and the def of
#                   the hoisted bound would then not dominate its use.
#   5. CONTROL      contract 2's disabled-pass arm is the negative control.
#                   Without it, a pass that never fired would also show a
#                   single load outside the loop and pass vacuously.
#   6. IDEMPOTENCE  with CCC_DEBUG_LOOP_PREHEADER set, the fixpoint's second
#                   pass over the same header reports "already has a
#                   dedicated preheader" and inserts nothing further. A pass
#                   that kept inserting would either loop forever or emit a
#                   chain of empty blocks.
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

# ── 2b. magnitude: the hoist must actually reduce in-loop memory traffic ──
# Contract 2 asserts a DIRECTION (the load moved out). That is necessary but
# not sufficient: a transform could hoist the load and simultaneously push
# other traffic INTO the loop -- a spill, a rematerialised address, a widened
# accumulator slot -- and still satisfy contract 2 while making the steady
# state slower. Counting every memory operand in the loop body pins the
# magnitude instead, so the pass has to leave the loop strictly lighter.
# Measured on this shape: 2 -> 1 (the p->nUsed load leaves; the accumulator's
# own traffic stays). Asserted as an exact equality, not a <=, because a
# number that silently drifts is a number nobody is watching.
mem_operand='^[[:space:]]*[a-z][a-z0-9]*[[:space:]].*\(%r'
on_mem=$(echo "$on_loop"  | grep -Ec "$mem_operand")
off_mem=$(echo "$off_loop" | grep -Ec "$mem_operand")
if [ "$on_mem" -eq 1 ] && [ "$off_mem" -eq 2 ]; then
    echo "ok: in-loop memory operands go 2 -> 1 (the hoist is a net reduction)"
elif [ "$on_mem" -ge "$off_mem" ]; then
    fail "the pass did not reduce in-loop memory traffic ($off_mem -> $on_mem): \
contract 2's direction held but the steady state is not lighter"
else
    fail "in-loop memory operand counts moved off the pinned 2 -> 1 \
(got $off_mem -> $on_mem); if this is a deliberate improvement, re-measure and \
update the expectation rather than widening it to a range"
fi

# ── 3. soundness: the load must not precede the NULL early return ───────
on_body=$(fn_body guarded_sum "$td/on.s")
ret_ln=$(echo "$on_body" | grep -nE '^[[:space:]]*ret' | sed -n '1,1p' | cut -d: -f1)
ld_ln=$(echo "$on_body"  | grep -nE "$base_only"       | sed -n '1,1p' | cut -d: -f1)
if [ -n "$ret_ln" ] && [ -n "$ld_ln" ] && [ "$ret_ln" -lt "$ld_ln" ]; then
    echo "ok: the hoisted load sits after the p==0 early return (line $ld_ln > $ret_ln)"
elif [ -z "$ld_ln" ]; then
    fail "no base-register load found in guarded_sum at all"
else
    fail "the hoisted load precedes the p==0 return (load@$ld_ln, ret@$ret_ln): \
the NULL path would dereference"
fi

# ── 4. scope: every shape the pass must refuse is left untouched ───────
for fn in invariant_ptr switch_entered computed_goto already_dedicated; do
    if diff <(fn_body "$fn" "$td/on.s") <(fn_body "$fn" "$td/off.s") >/dev/null; then
        echo "ok: $fn is byte-identical (pass correctly declined)"
    else
        fail "$fn changed between the two arms: the pass inserted where it \
must not"
        diff <(fn_body "$fn" "$td/on.s") <(fn_body "$fn" "$td/off.s") | sed -n '1,8p' >&2
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

exit $rc
