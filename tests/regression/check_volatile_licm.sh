#!/usr/bin/env bash
# LICM must not hoist a volatile load out of its loop (C11 5.1.2.3).
#
#   1. RUNTIME     the battery computes the right values with LICM on and
#                  with LICM off (CCC_DISABLE_PASSES=licm). This guards
#                  against a miscompilation, which is NOT the bug in
#                  question -- see the note in volatile_licm.c.
#   2. STRUCTURAL  in sum_volatile's assembly the load of g_volatile sits
#                  INSIDE the loop; in sum_plain's assembly the load of
#                  g_plain sits OUTSIDE it. This is the actual detector:
#                  hoisting a volatile load changes the number of
#                  observable accesses while producing the same answer, so
#                  no stdout comparison can see it.
#   3. CONTROL     contract 2's second half is the negative control. A
#                  compiler that hoisted nothing would also leave the
#                  volatile load in place, so the test proves LICM is
#                  really running by showing it really hoists the plain
#                  load. Without it, contracts 1 and 2 pass vacuously.
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
src=tests/regression/volatile_licm.c
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT

[[ -x $ccc ]] || { echo "check_volatile_licm: lccc not found at $ccc" >&2; exit 1; }

march=$("$ccc" -march=x86-64-v3 -E -x c /dev/null -o /dev/null 2>/dev/null \
    && echo "-march=x86-64-v3" || echo "")

# ── 1. runtime, LICM on and off ─────────────────────────────────────────
for mode in on off; do
    if [ "$mode" = on ]; then
        unset CCC_DISABLE_PASSES
    else
        export CCC_DISABLE_PASSES=licm
    fi
    if ! "$ccc" -O2 $march "$src" -o "$td/rt-$mode" 2>"$td/cc-$mode.log"; then
        echo "FAIL: compile failed with LICM $mode" >&2
        sed -n '1,20p' "$td/cc-$mode.log" >&2
        exit 1
    fi
    if ! out=$("$td/rt-$mode"); then
        echo "FAIL: program exited non-zero with LICM $mode" >&2
        exit 1
    fi
    if [ "$out" != "volatile_licm: ok" ]; then
        echo "FAIL: wrong output with LICM $mode: $out" >&2
        exit 1
    fi
done
unset CCC_DISABLE_PASSES
echo "  ok: runtime correct with LICM on and off"

if [ -z "$march" ]; then
    echo "OK: volatile LICM runtime contracts (no x86-64-v3 here; asm section skipped)"
    exit 0
fi

# ── 2-3. structural ─────────────────────────────────────────────────────
"$ccc" -O2 $march -S "$src" -o "$td/v.s"

rc=0
# `cmd || rc=$?` is immune to `set -e`; a bare `python3 ... <<PY` that
# exits non-zero would abort the script here and strand rc=$? below.
python3 - "$td/v.s" <<'PY' || rc=$?
import re, sys

asm = open(sys.argv[1], errors="replace").read().splitlines()

def scoped(fn):
    """The asm slice for `fn`: its label up to the next function label."""
    out, inside = [], False
    for line in asm:
        if re.match(rf"^{re.escape(fn)}:\s*$", line):
            inside = True
            continue
        if inside and re.match(r"^[a-zA-Z_][a-zA-Z0-9_]*:\s*$", line):
            break
        if inside:
            out.append(line)
    return out

def loop_span(body):
    """(start, end) line range of the first backward branch, else None.

    A backward jump is the loop: its target is the header and the jump is
    the latch. Taking the LAST backward jump keeps this correct for a
    function with several loops -- we analyse the one that encloses the
    load we care about, which is the innermost/only loop here.
    """
    heads = {i: m.group(1)
             for i, l in enumerate(body)
             if (m := re.match(r"^\.LBB(\d+):\s*$", l.strip()))}
    best = None
    for i, l in enumerate(body):
        m = re.match(r"^\s*j[a-z]+\s+\.LBB(\d+)", l)
        if not m:
            continue
        tgt = m.group(1)
        for h_i, h in heads.items():
            if h == tgt and h_i < i:
                if best is None or h_i > best[0]:
                    best = (h_i, i)
    return best

def load_line(body, sym):
    """Index of the first instruction that reads `sym` from memory."""
    for i, l in enumerate(body):
        if sym in l and re.search(r"\b(mov|add|cmp|imul|sub)[a-z]*\b", l):
            if re.search(rf"{re.escape(sym)}\(%rip\)|\b{re.escape(sym)}\b", l):
                return i
    return None

fail = 0
def check(ok, msg):
    global fail
    print(("  ok: " if ok else "  FAIL: ") + msg)
    if not ok:
        fail = 1

# --- volatile load must stay INSIDE the loop (do-while shape)
bv = scoped("sum_volatile_dowhile")
span = loop_span(bv)
lv = load_line(bv, "g_volatile")
if span is None:
    check(False, "sum_volatile: no backward branch found -- loop not identified")
elif lv is None:
    check(False, "sum_volatile: no load of g_volatile found in the function")
else:
    inside = span[0] < lv < span[1]
    check(inside,
          "sum_volatile: load of g_volatile is inside the loop "
          f"(load@{lv}, loop {span[0]}..{span[1]}) -- "
          + ("not hoisted" if inside else "HOISTED OUT OF THE LOOP"))
    if not inside:
        print("      function asm:")
        for i, l in enumerate(bv):
            print(f"      {i:3d} {l}")

# --- plain load must be hoisted OUT of the loop (negative control)
bp = scoped("sum_plain_dowhile")
spanp = loop_span(bp)
lp = load_line(bp, "g_plain")
if spanp is None:
    check(False, "sum_plain: no backward branch found -- loop not identified")
elif lp is None:
    check(False, "sum_plain: no load of g_plain found in the function")
else:
    hoisted = not (spanp[0] < lp < spanp[1])
    check(hoisted,
          "sum_plain: load of g_plain is OUTSIDE the loop "
          f"(load@{lp}, loop {spanp[0]}..{spanp[1]}) -- "
          + ("hoisted, so LICM is running" if hoisted
             else "NOT hoisted: LICM is not running, so the volatile "
                  "contract above proves nothing"))
    if not hoisted:
        print("      function asm:")
        for i, l in enumerate(bp):
            print(f"      {i:3d} {l}")

sys.exit(fail)
PY
if [ "$rc" -ne 0 ]; then
    echo "FAIL: volatile LICM structural contracts" >&2
    exit 1
fi
echo "PASS: volatile LICM (runtime both arms, load in loop, plain load hoisted)"
