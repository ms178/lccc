#!/usr/bin/env bash
# ============================================================================
# check_layout_tie_rule.sh — the dual-layout tie rule is wired, is load-bearing,
# and does not change what the program computes.
#
# WHY A GATE
# ----------
# `adopt_chain_layout` decides between two complete block orders for a function.
# Its unit tests pin the predicate; nothing pinned the RULE. A refactor that
# stops calling the predicate, a fixture that quietly stops tying, or a rule
# whose "win" is an artefact of one binary are all green in the unit tests and
# all wrong in production. This gate compiles the fixture under both arms and
# checks the emitted code, so the rule is measured where it acts.
#
# WHAT IS PINNED
# --------------
#   1. THE FIXTURE TIES. `CCC_DEBUG_CHAIN=1` must report at least one site with
#      chain_count == orig_count. Zero ties means the fixture moved out of the
#      tie bucket and the rest of this gate would pass vacuously.
#   2. THE RULE IS LIVE. The default arm and the `CCC_LAYOUT_TIE_ORIG=1` arm must
#      emit different assembly. Identical output means the switch — and with it
#      the rule — stopped acting.
#   3. IT BUYS BRANCHES. On a tie the chain order falls into the shared latch
#      where the append order needs a conditional jump to reach it, so the
#      default arm must emit strictly fewer conditional jumps and the same
#      number of unconditional ones. (Measured when the rule landed: 2 -> 0
#      conditional jumps, 13 -> 13 `jmp`.) The `jmp` count is pinned equal on
#      purpose: a rule that "wins" by deleting a branch would be a different
#      change, and this gate exists to keep it from being smuggled in here.
#   4. NOTHING ELSE MOVES. Both arms are executed and their stdout must match
#      the GCC oracle's, because a block order is a performance decision and
#      never a licence to change a result.
#
# The fixture is `expat_xml_scan` — a real scanner whose name-character arm and
# loop latch share a successor, which is exactly the shape where the two orders
# disagree. It is a benchmark program rather than a synthetic `.c` because the
# tie only appears after the full -O2 pipeline has inlined and folded it.
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
GCC=${GCC:-gcc}
SRC=$root/tests/benchmark/programs/expat_xml_scan.c
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fail=0

say() { printf '%s\n' "$*"; }
bad() { printf 'FAIL: %s\n' "$*" >&2; fail=1; }

[[ -x $CCC ]] || { printf 'check_layout_tie_rule: lccc not found at %s\n' "$CCC" >&2; exit 1; }
[[ -f $SRC ]] || { printf 'check_layout_tie_rule: fixture missing: %s\n' "$SRC" >&2; exit 1; }

# 1. The fixture must still reach the tie path.
ties=$(CCC_DEBUG_CHAIN=1 "$CCC" -O2 -S "$SRC" -o /dev/null 2>&1 |
    sed -nE 's/.*= ([0-9]+) \(stack [0-9]+\).*= ([0-9]+) \(stack [0-9]+\).*/\1 \2/p' |
    awk '$1 == $2 {n++} END {print n + 0}')
if [[ $ties -lt 1 ]]; then
    bad "fixture produced no exact layout tie (CCC_DEBUG_CHAIN reported none); the gate would pass vacuously"
else
    say "tie sites in fixture: $ties"
fi

# 2/3. Emit both arms and compare branch structure.
"$CCC" -O2 -S "$SRC" -o "$tmp/tie.s"
CCC_LAYOUT_TIE_ORIG=1 "$CCC" -O2 -S "$SRC" -o "$tmp/orig.s"

if cmp -s "$tmp/tie.s" "$tmp/orig.s"; then
    bad "the tie rule is inert: default and CCC_LAYOUT_TIE_ORIG=1 emit identical assembly"
fi

# `grep -c` exits 1 on a zero count, which under `pipefail`+`set -e` would
# abort the gate instead of feeding the assertion it exists for.
count() { # count <file> <mnemonic-regex>
    grep -E "^[[:space:]]+$2[[:space:]]" "$1" | wc -l || true
}
cond() { # conditional branches = jumps that are not `jmp`
    grep -E '^[[:space:]]+j[a-z]+[[:space:]]' "$1" 2>/dev/null |
        grep -vcE '^[[:space:]]+jmp[[:space:]]' || true
}
tie_cond=$(cond "$tmp/tie.s")
orig_cond=$(cond "$tmp/orig.s")
tie_jmp=$(count "$tmp/tie.s" 'jmp')
orig_jmp=$(count "$tmp/orig.s" 'jmp')
say "conditional branches: tie=$tie_cond historical=$orig_cond; jmp: tie=$tie_jmp historical=$orig_jmp"
if [[ $tie_cond -ge $orig_cond ]]; then
    bad "the tie rule did not remove a conditional branch ($tie_cond vs $orig_cond)"
fi
if [[ $tie_jmp -ne $orig_jmp ]]; then
    bad "unconditional jump count moved ($tie_jmp vs $orig_jmp); the rule must only change which branches are taken"
fi

# 4. Both arms must agree with the oracle on the program's output.
"$CCC" -O2 "$SRC" -o "$tmp/tie.bin"
CCC_LAYOUT_TIE_ORIG=1 "$CCC" -O2 "$SRC" -o "$tmp/orig.bin"
"$tmp/tie.bin" >"$tmp/tie.out"
"$tmp/orig.bin" >"$tmp/orig.out"
if ! cmp -s "$tmp/tie.out" "$tmp/orig.out"; then
    bad "the two block orders disagree on the program's output"
fi
if command -v "$GCC" >/dev/null 2>&1; then
    "$GCC" -O2 "$SRC" -o "$tmp/gcc.bin"
    "$tmp/gcc.bin" >"$tmp/gcc.out"
    if ! cmp -s "$tmp/tie.out" "$tmp/gcc.out"; then
        bad "the tie arm disagrees with the GCC oracle's output"
    fi
else
    say "NOTE: no gcc oracle on PATH; oracle comparison skipped"
fi

if [[ $fail -ne 0 ]]; then
    printf 'check_layout_tie_rule: FAILED\n' >&2
    exit 1
fi
say "check_layout_tie_rule: PASS"
