#!/usr/bin/env bash
# T5 of engineering/FOLLOWUP-2026-10-18-vellvm-vir-transplant.md: a bounded
# fixpoint loop that stops because its round budget ran out (while the last
# round still changed the IR) must be observable, not silent.
#
# Two checks, both fail closed:
#   1. Static: every `round_cap::Site` variant is wired to exactly one
#      `note_cap_hit(round_cap::Site::<V>)` call in src/passes, and every
#      call names a declared variant. A new site without a call, or a call
#      whose counter was deleted, fails here.
#   1b. Pipeline-exit coverage: every early exit of `run_passes` that can
#      have counters set must call `round_cap::report()`, including the -O0
#      tier (its always-inline second-pass budget is a correctness budget).
#      The only exempt exit is `pass_disabled("all")`, where no pass runs
#      and every counter is provably zero.
#   2. Behavioral witnesses: `cross_pr_redteam.c` exhausts backedge_pre's
#      3-round budget and `fortify_open_va_pack.c` exhausts the ordinary
#      inline budget, both at -O2 (2 654-file corpus sweep, see the
#      follow-up). Under CCC_TIME_PASSES=1 the compiler must print the
#      matching `round_cap <site>` line; with the variable unset it must
#      print nothing. A cap-hit edit that drops the report, or a cap change
#      that removes the witness, fails here and must be re-measured rather
#      than silently relaxed.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT"
CCC=${CCC:-target/fastbuild/lccc}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-roundcap.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

fail=0
die() { echo "FAIL: $*" >&2; fail=1; }

# 1. Static wiring ---------------------------------------------------------
decl=$(sed -n '/^pub(crate) enum Site {/,/^}/p' src/passes/round_cap.rs \
    | sed -n 's/^    \([A-Za-z0-9]*\),$/\1/p')
[ -n "$decl" ] || die "could not parse the Site enum in src/passes/round_cap.rs"
calls=$(grep -rhoE 'round_cap::note_cap_hit\(round_cap::Site::[A-Za-z0-9]+\)' src/passes \
    | sed -E 's/.*Site::([A-Za-z0-9]+)\)/\1/' | sort)
for v in $decl; do
    n=$(grep -c "^$v\$" <<<"$calls" || true)
    [ "$n" -eq 1 ] || die "Site::$v has $n note_cap_hit call(s), want exactly 1"
done
while read -r c; do
    [ -n "$c" ] || continue
    grep -qx "$c" <<<"$decl" || die "note_cap_hit names undeclared Site::$c"
done <<<"$calls"
echo "static: $(wc -w <<<"$decl" | tr -d ' ') sites wired"

# 1b. Pipeline-exit coverage ------------------------------------------------
# Fail closed: parse run_passes, find every unit `return;`, and require that
# a round_cap::report() call dominates it -- except the one exit inside the
# pass_disabled("all") guard (no pass can have run, counters provably zero).
# Uses the same poor-man's parser style as other repo gates; anything it
# cannot prove is a failure, never a silent skip.
# The driver takes the mod.rs path as argv[1] so the wiring invariants can be
# re-run against any tree copy (the gate's own mutation tests use that).
if ! python3 - src/passes/mod.rs <<'PYEOF'
import re
import sys

src = open(sys.argv[1], encoding="utf-8").read()
m = re.search(r"\npub\(crate\) fn run_passes\(", src)
if m is None:
    sys.exit("FAIL: could not find run_passes in src/passes/mod.rs")
i = src.index("{", m.end())
depth = 0
end = None
for j in range(i, len(src)):
    if src[j] == "{":
        depth += 1
    elif src[j] == "}":
        depth -= 1
        if depth == 0:
            end = j
            break
if end is None:
    sys.exit("FAIL: could not brace-match run_passes")
lines = src[i:end + 1].split("\n")

fail = 0
exempt = 0
returns = []
for n, line in enumerate(lines, 1):
    # Comment-stripped code only. Match the unit-return TOKEN, not the
    # whole line: a fullmatch on `\s*return;\s*` silently missed one-line
    # forms (`if (c) { return; }`, `} else { return; }`), so a future
    # compact early exit could skip the report without this gate noticing.
    # `return EXPR;` stays out of scope: it can only appear inside a
    # nested closure in this unit-returning function (the i686 walk below
    # has one), and a closure return never exits the pipeline.
    code = line.split("//")[0]
    if re.search(r"\breturn;", code):
        window = "\n".join(lines[max(0, n - 9):n])
        if 'pass_disabled(&disabled, "all")' in window:
            exempt += 1
            continue
        if "round_cap::report()" not in window:
            print(f"FAIL: run_passes body line {n}: 'return' without a "
                  f"round_cap::report() dominating it", file=sys.stderr)
            fail = 1
        returns.append(n)

# The fall-through at the end of the function is an exit too.  The window is
# wider than the per-return one: it still fails on a deleted report(), but
# tolerates a few lines of trailing dumps.
tail = "\n".join(lines[-15:])
if "round_cap::report()" not in tail:
    print("FAIL: the main fall-through exit of run_passes lacks a "
          "round_cap::report()", file=sys.stderr)
    fail = 1
if exempt != 1:
    print(f"FAIL: expected exactly 1 exempt pass_disabled(\"all\") exit, "
          f"found {exempt}", file=sys.stderr)
    fail = 1
if fail == 0:
    print(f"exit-coverage: {len(returns)} pass-bearing early exits report, "
          f"1 exempt (pass-disabled), fall-through reports")
sys.exit(fail)
PYEOF
then
    die "pipeline-exit coverage check failed"
fi

# 2. Behavioral witnesses ---------------------------------------------------
# Each pair is "<source> <expected report prefix>", measured on the 2 654-file
# corpus at -O2 (see the follow-up document). The report must appear under
# CCC_TIME_PASSES=1 and must not appear without it.
witness() {
    local src=$1 want=$2
    CCC_TIME_PASSES=1 "$CCC" -O2 -c "$src" -o "$tmp/on.o" 2>"$tmp/on.err" \
        || { die "lccc failed on $src with CCC_TIME_PASSES=1"; return; }
    grep -q "^\[PASS\] round_cap $want: [0-9][0-9]* hit(s)\$" "$tmp/on.err" \
        || { die "no 'round_cap $want' report for $src under CCC_TIME_PASSES=1"; return; }
    "$CCC" -O2 -c "$src" -o "$tmp/off.o" 2>"$tmp/off.err" \
        || { die "lccc failed on $src without CCC_TIME_PASSES"; return; }
    grep -q 'round_cap' "$tmp/off.err" && die "round_cap output leaked without CCC_TIME_PASSES for $src"
    echo "witness: $want on $src"
}
witness tests/regression/cross_pr_redteam.c backedge_pre
witness tests/regression/fortify_open_va_pack.c inline_ordinary_rounds

if [ "$fail" -ne 0 ]; then
    exit 1
fi
echo "OK check_round_cap_observability"
