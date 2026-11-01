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
#   3. IT BUYS TAKEN TRANSFERS. On a tie the chain order falls into the shared
#      latch where the append order needs a jump to reach it. Both orders emit
#      the same branches -- the rule changes which edge falls through, not how
#      many jumps exist -- so the win is dynamic and the gate measures it so:
#      under Callgrind (`--collect-jumps=yes --dump-instr=yes`) the default arm
#      must execute strictly fewer taken transfers (taken conditional jumps
#      plus unconditional jumps) than the historical arm, and the static `jmp`
#      count must be equal. Only transfers inside the fixture's OWN object are
#      counted: the dynamic loader and libc are identical code in both arms
#      but their jump counts drift by one or two between otherwise identical
#      runs, which once made this clause pass or fail on noise alone. The `jmp` count is pinned equal on purpose: a rule
#      that "wins" by deleting a branch would be a different change, and this
#      gate exists to keep it from being smuggled in here.
#      This clause used to count conditional jumps statically with a pattern
#      that required leading whitespace, and the tie arm's `emit_jcc_block`
#      jumps were then emitted at column 0: its "2 -> 0" on expat_xml_scan was
#      those two lines going uncounted. Counted at any indentation the arms
#      emit 39 and 39. Measured dynamically on ms178/lccc main (219f74aeb) the
#      rule is real: 17,287,668 -> 9,229,551 executed `jmp`, 647.5M -> 639.5M
#      Ir on expat_xml_scan with identical output.
#   4. NOTHING ELSE MOVES. Both arms are executed and their stdout must match
#      the GCC oracle's, because a block order is a performance decision and
#      never a licence to change a result.
#
# THE FIXTURE
# -----------
# A tie only appears after the full -O2 pipeline has inlined and folded a real
# program, and exact ties have become rare: over the 86 benchmark programs and
# the 1,220-file regression corpus the pipeline reaches four, all on cold paths
# (chacha20_block's and fp_memfold_stencil5's `main`, two regression files),
# where both arms execute the same transfers. A cold tie proves nothing, so the
# fixture is the one hot tie the rule was measured on: `expat_xml_scan`, whose
# name-character arm and loop latch share a successor, compiled with
# `CCC_DISABLE_PASSES=jump_thread`. The late `jump_thread` threads its UTF-8
# width phi (27 -> 34 blocks), which resolves the tie; switching it off
# restores the shape without touching the layout code under test, which runs
# unchanged on whatever CFG it is given. Measured on this configuration the
# rule removes 575,581 of 96,973,245 executed taken transfers in the program
# (-0.59 %; 621.05M -> 620.48M Ir) with output identical to GCC's.
# There is deliberately no fall-back candidate: if the fixture stops tying,
# clause 1 fails rather than the gate drifting onto a cold tie. Find a new hot
# tie by running `CCC_DEBUG_CHAIN=1` over the corpus (a `[LAYOUT]` line with
# equal totals) and measuring both arms as below; LAYOUT_TIE_SRC and
# LAYOUT_TIE_ENV (whitespace-separated VAR=value words) override the fixture.
#
# Valgrind is a gate dependency (scripts/ci_provision_toolchain.sh installs it
# with libc6-dbg); without it clause 3 cannot be measured and the gate fails
# rather than passing on the static clauses alone.
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
GCC=${GCC:-gcc}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fail=0

say() { printf '%s\n' "$*"; }
bad() { printf 'FAIL: %s\n' "$*" >&2; fail=1; }

[[ -x $CCC ]] || { printf 'check_layout_tie_rule: lccc not found at %s\n' "$CCC" >&2; exit 1; }
command -v valgrind >/dev/null 2>&1 ||
    { printf 'check_layout_tie_rule: valgrind is required to measure taken transfers\n' >&2; exit 1; }
programs=$root/tests/benchmark/programs
SRC=${LAYOUT_TIE_SRC:-$programs/expat_xml_scan.c}
read -r -a fixture_env <<<"${LAYOUT_TIE_ENV-CCC_DISABLE_PASSES=jump_thread}"
lccc() { # lccc [VAR=value...] <compiler> <args>: run under the fixture's environment
    env "${fixture_env[@]}" "$@"
}

tie_sites() { # tie_sites <src>: sites where chain_count == orig_count
    lccc CCC_DEBUG_CHAIN=1 "$CCC" -O2 -S "$1" -o /dev/null 2>&1 |
        sed -nE 's/.*= ([0-9]+) \(stack [0-9]+\).*= ([0-9]+) \(stack [0-9]+\).*/\1 \2/p' |
        awk '$1 == $2 {n++} END {print n + 0}'
}

# 1. The fixture must still reach the tie path.
[[ -f $SRC ]] || { printf 'check_layout_tie_rule: fixture missing: %s\n' "$SRC" >&2; exit 1; }
ties=$(tie_sites "$SRC")
say "fixture: $(basename "$SRC") [${fixture_env[*]}], tie sites: $ties"
if [[ $ties -lt 1 ]]; then
    bad "the fixture no longer produces an exact layout tie (CCC_DEBUG_CHAIN reported none); the gate would pass vacuously -- see THE FIXTURE"
    printf 'check_layout_tie_rule: FAILED\n' >&2
    exit 1
fi

# 2. Emit both arms; they must differ.
lccc "$CCC" -O2 -S "$SRC" -o "$tmp/tie.s"
lccc CCC_LAYOUT_TIE_ORIG=1 "$CCC" -O2 -S "$SRC" -o "$tmp/orig.s"

if cmp -s "$tmp/tie.s" "$tmp/orig.s"; then
    bad "the tie rule is inert: default and CCC_LAYOUT_TIE_ORIG=1 emit identical assembly"
fi

# Instructions are counted at any indentation: the measurement must follow the
# code, not the emitter's layout. `grep -c` exits 1 on a zero count, which
# under `pipefail`+`set -e` would abort the gate, hence `|| true`.
count() { # count <asm> <mnemonic>
    grep -cE "^[[:space:]]*$2[[:space:]]" "$1" || true
}
tie_jmp=$(count "$tmp/tie.s" 'jmp')
orig_jmp=$(count "$tmp/orig.s" 'jmp')
if [[ $tie_jmp -ne $orig_jmp ]]; then
    bad "unconditional jump count moved ($tie_jmp vs $orig_jmp); the rule must only change which branches are taken"
fi

# Executed taken transfers inside one binary's own object: Callgrind's
# per-instruction jump records, `jcnd=<taken>/<executed>` and `jump=<count>`,
# summed while the current `ob=` is the binary itself. Callgrind compresses
# names: the first `ob=(N) path` binds N, later lines repeat only `ob=(N)`.
taken_transfers() { # taken_transfers <binary> <callgrind-out>
    valgrind --tool=callgrind --collect-jumps=yes --dump-instr=yes \
        --callgrind-out-file="$2" "$1" >/dev/null 2>&1
    awk -v own="$1" '
        /^ob=/ {
            id = $0; sub(/^ob=\(/, "", id); sub(/\).*/, "", id)
            rest = $0; sub(/^ob=\([0-9]+\) ?/, "", rest)
            if (rest != "") name[id] = rest
            cur = name[id]
            next
        }
        cur == own && /^(jcnd|jump)=/ { split($0, f, /[=\/ ]/); n += f[2] }
        END { print n + 0 }' "$2"
}

# Build and run both arms: clause 3 measures them, clause 4 checks their output.
lccc "$CCC" -O2 "$SRC" -o "$tmp/tie.bin"
lccc CCC_LAYOUT_TIE_ORIG=1 "$CCC" -O2 "$SRC" -o "$tmp/orig.bin"
"$tmp/tie.bin" >"$tmp/tie.out"
"$tmp/orig.bin" >"$tmp/orig.out"
if ! cmp -s "$tmp/tie.out" "$tmp/orig.out"; then
    bad "the two block orders disagree on the program's output"
fi

# 4. Both arms agree with each other and with the GCC oracle.
if command -v "$GCC" >/dev/null 2>&1; then
    "$GCC" -O2 "$SRC" -o "$tmp/gcc.bin"
    "$tmp/gcc.bin" >"$tmp/gcc.out"
    if ! cmp -s "$tmp/tie.out" "$tmp/gcc.out"; then
        bad "the tie arm disagrees with the GCC oracle's output"
    fi
else
    say "NOTE: no gcc oracle on PATH; oracle comparison skipped"
fi

# 3 (dynamic). The default arm takes strictly fewer transfers.
tie_taken=$(taken_transfers "$tmp/tie.bin" "$tmp/tie.cg")
orig_taken=$(taken_transfers "$tmp/orig.bin" "$tmp/orig.cg")
say "jmp: tie=$tie_jmp historical=$orig_jmp; executed taken transfers: tie=$tie_taken historical=$orig_taken"
if [[ $tie_taken -eq 0 ]]; then
    tie_objects=$(grep '^ob=' "$tmp/tie.cg" | sort -u | head -n 8 | tr '\n' ';' || true)
    tie_records=$(grep -m 8 -E '^(jcnd|jump)=' "$tmp/tie.cg" | tr '\n' ';' || true)
    say "FAIL: Callgrind tie-arm objects: $tie_objects"
    say "FAIL: Callgrind tie-arm transfer records: $tie_records"
fi
if [[ $tie_taken -eq 0 || $orig_taken -eq 0 ]]; then
    bad "callgrind recorded no jumps (tie=$tie_taken historical=$orig_taken); the measurement did not run"
elif [[ $tie_taken -ge $orig_taken ]]; then
    bad "the tie rule did not remove a taken transfer ($tie_taken vs $orig_taken)"
fi

if [[ $fail -ne 0 ]]; then
    printf 'check_layout_tie_rule: FAILED\n' >&2
    exit 1
fi
say "check_layout_tie_rule: PASS"
