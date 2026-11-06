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


def check(lines):
    """Every pass-bearing early exit owns exactly one round_cap::report().

    A fixed 9-line lookback was NOT a dominance proof: adding four lines of
    formatting to a debug dump moved an otherwise dominating report() out of
    the window and turned a correct tree into a red CI lane.  Distance in
    lines is not a control-flow property.  Instead partition the function
    body at each early exit and require exactly one report per region, with
    one further report for the fall-through.  With N pass-bearing exits the
    count must therefore be N+1, so a deleted, duplicated or misplaced report
    fails regardless of how many lines anybody inserts -- while a report
    genuinely in a different region cannot be borrowed by a neighbour.
    Comments are stripped so a magic string in a comment cannot satisfy it.
    """
    problems = []
    exempt = 0
    returns = []
    for n, line in enumerate(lines, 1):
        # Match the unit-return TOKEN, not the whole line: a fullmatch on
        # `\s*return;\s*` silently missed compact forms such as
        # `if (c) { return; }`.  `return EXPR;` can only be a nested closure
        # return in this unit-returning function, never a pipeline exit.
        code = line.split("//")[0]
        if not re.search(r"\breturn;", code):
            continue
        j = n - 2
        while j >= 0 and not lines[j].strip():
            j -= 1
        guard = code + ("\n" + lines[j] if j >= 0 else "")
        if 'pass_disabled(&disabled, "all")' in guard:
            exempt += 1
            continue
        returns.append(n)
    reports = [n for n, l in enumerate(lines, 1)
               if "round_cap::report()" in l.split("//")[0]]
    if exempt != 1:
        problems.append('expected exactly 1 exempt pass_disabled("all") '
                        f'exit, found {exempt}')
    if len(reports) != len(returns) + 1:
        problems.append(f"expected {len(returns) + 1} report() calls for "
                        f"{len(returns)} pass-bearing exits + fall-through, "
                        f"found {len(reports)}")
    prev = 0
    for n in returns:
        owned = [r for r in reports if prev < r < n]
        if len(owned) != 1:
            problems.append(f"run_passes body line {n}: {len(owned)} "
                            "report() calls dominate this exit, want 1")
        prev = n
    tail = "\n".join(l.split("//")[0] for l in lines[-15:])
    if "round_cap::report()" not in tail:
        problems.append("the main fall-through exit of run_passes lacks a "
                        "round_cap::report()")
    if prev and not any(r > prev for r in reports):
        problems.append("no report() after the last early exit")
    return problems


def self_test():
    """Prove the checker accepts distance but rejects real regressions."""
    filler = ["        // formatting-only line"] * 40
    good = (['    if pass_disabled(&disabled, "all") { return; }',
             '    if time_passes { round_cap::report(); }',
             '    return;']
            + filler
            + ['    if time_passes { round_cap::report(); }',
               '    return;',
               '    if time_passes { round_cap::report(); }']
            + ["    // tail"] * 4)
    cases = [
        ("40 formatting lines between report and exit", good, True),
        ("a report hidden in a comment",
         [l.replace('round_cap::report();', '// round_cap::report();')
          if i == 1 else l for i, l in enumerate(good)], False),
        ("a deleted report",
         [l for i, l in enumerate(good) if i != 43], False),
        ("a duplicated report", good[:2] + good[1:2] + good[2:], False),
        ("the exempt guard removed",
         ['    return;' if i == 0 else l for i, l in enumerate(good)], False),
        ("compact early exit with a report", good, True),
    ]
    failures = []
    for label, lines, want_ok in cases:
        got_ok = not check(lines)
        if got_ok != want_ok:
            failures.append(f"{label}: accepted={got_ok}, want {want_ok}")
    if failures:
        for f in failures:
            print(f"FAIL self-test: {f}", file=sys.stderr)
        return 1
    print(f"exit-coverage self-test: {len(cases)} mutations classified")
    return 0


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

rc = self_test()
problems = check(lines)
for p in problems:
    print(f"FAIL: {p}", file=sys.stderr)
    rc = 1
if rc == 0:
    n_returns = sum(bool(re.search(r"\breturn;", l.split("//")[0]))
                    for l in lines)
    print(f"exit-coverage: {n_returns - 1} pass-bearing early exits report, "
          "1 exempt (pass-disabled), fall-through reports")
sys.exit(rc)
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
