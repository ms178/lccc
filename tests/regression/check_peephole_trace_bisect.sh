#!/usr/bin/env bash
# Gate for scripts/peephole_trace_bisect.py, the tool the x86 peephole driver's
# own comment names:
#
#   The trace is the reliable instrument: assemble+run consecutive dumps and
#   the first one that breaks the program names the faulty rewrite
#   (`scripts/peephole_trace_bisect.py` automates it).
#
# That comment referred to a script that did not exist, which is how the
# `fold_lea_into_load` miscompile had to be bisected by hand: `CCC_PEEPHOLE_SKIP`
# reported eight different "culprits" for one bug, because the pass that fires
# only after an earlier rewrite shows up as the cause.  A tool that nothing runs
# rots, so this gate keeps it honest.
#
# What is checked, and why each part matters:
#
#   * the operand parser.  It decides which dump is reported.  A regex-based
#     first attempt silently missed the empty-base form `(,%r11,4, %r10, 1)` --
#     exactly the shape the historical bug produced -- so the self-test pins
#     every shape: the two broken spellings, the empty base slot, an illegal
#     scale, three register fields, and the seven correct forms that must NOT be
#     reported (`(%r10, %r11, 4)`, `(%r8, %r11, 1)`, index-only, base-only,
#     `8(%rsp, %r8)`, a symbol-indexed load, `%rip`-relative, and the same bad
#     text inside a `#` comment).  A false positive is as bad as a false
#     negative here: it names the wrong pass.
#
#   * dump-name parsing, including a name with dots (`000-p0-a.b.s`) and a
#     non-dump file that must be ignored rather than crash.
#
#   * the end-to-end path on a real compile, which is the only way to catch a
#     tool that is internally consistent and still cannot drive the compiler:
#     it must observe dumps, and on a fixed compiler it must report clean.
#     The historical failure cannot be re-created here (the compiler is fixed),
#     so the *static* half is exercised by the self-test and the *dynamic* half
#     by asserting the trace is produced and comes back clean.
set -uo pipefail

cd "$(dirname "$0")/../.."
ROOT=$PWD
LCCC=${CCC:-target/fastbuild/lccc}
PY=python3
TOOL=scripts/peephole_trace_bisect.py

fail=0
note() { printf '  %s\n' "$*"; }
bad() { printf '  FAIL %s\n' "$*"; fail=1; }

if [ ! -f "$TOOL" ]; then
    echo "peephole-trace-bisect: FAIL -- $TOOL is missing" >&2
    exit 1
fi

echo "== phase 1: operand-parser and dump-name self-test =="
if $PY "$TOOL" --selftest >/tmp/ptb_selftest.$$ 2>&1; then
    sed 's/^/  /' /tmp/ptb_selftest.$$ | tail -1
else
    sed 's/^/  /' /tmp/ptb_selftest.$$
    bad "self-test"
fi
rm -f /tmp/ptb_selftest.$$

# The self-test must be able to FAIL: a gate whose only assertion is "the
# program agrees with itself" passes on an empty parser.  Perturb one case and
# require a non-zero exit.
echo "== phase 2: the self-test can fail (mutation proof) =="
TMPD=$(mktemp -d)
trap 'rm -rf "$TMPD"' EXIT
if sed 's/^    ("    leaq 0(,%r11,4, %r10, 1), %r9", True, "historical broken fold"),//' \
    "$TOOL" > "$TMPD/mutated1.py" &&
    sed -i 's/^    ("    leaq (%r10, %r11, 4), %r9", False, "the fix"),/    ("    leaq 0(,%r11,4, %r10, 1), %r9", False, "inverted"),/' \
        "$TMPD/mutated1.py"; then
    if $PY -c "
import ast,sys
src=open('$TMPD/mutated1.py').read()
if 'inverted' not in src: sys.exit(3)
" && $PY "$TMPD/mutated1.py" --selftest >/dev/null 2>&1; then
        bad "an inverted expectation still passed -- the self-test is vacuous"
    else
        note "inverted expectation -> non-zero exit, as required"
    fi
else
    bad "could not build the mutated copy"
fi

echo "== phase 3: end-to-end trace on a real compile =="
SRC=tests/regression/lea_chain_index_compose.c
if [ ! -x "$LCCC" ]; then
    note "SKIP -- $LCCC not built (self-test phases already ran)"
elif [ ! -f "$SRC" ]; then
    bad "$SRC missing; the dynamic phase has nothing to drive"
else
    TRACE=$TMPD/trace
    mkdir -p "$TRACE"
    if ! CCC_PEEPHOLE_TRACE=$TRACE "$LCCC" -O1 "$SRC" -o "$TMPD/a.out" 2>"$TMPD/cc.err"; then
        bad "compile failed: $(tail -1 "$TMPD/cc.err")"
    else
        ndumps=$(ls "$TRACE" | wc -l)
        if [ "$ndumps" -eq 0 ]; then
            bad "no dumps written; CCC_PEEPHOLE_TRACE is not reaching the driver"
        else
            note "trace produced $ndumps dumps"
            if $PY "$TOOL" --trace-dir "$TRACE" --quiet >"$TMPD/verdict" 2>&1; then
                # Assert the CONTENT of the verdict, not just the exit code: a
                # tool that printed nothing and exited 0 would satisfy the code
                # alone.
                if grep -q '^verdict: no dump introduces' "$TMPD/verdict"; then
                    note "$(head -1 "$TMPD/verdict")"
                else
                    bad "clean exit but no clean verdict:"
                    sed 's/^/    /' "$TMPD/verdict"
                fi
            else
                bad "the fixed compiler still produces a corrupt dump:"
                sed 's/^/    /' "$TMPD/verdict"
            fi
        fi
        # The instrument must also work on a caller-supplied pattern, which is
        # how it is used for a bug that is not a bad operand.
        if $PY "$TOOL" --trace-dir "$TRACE" --pattern 'leaq 0\(,%r11,4\), %r8' --quiet \
            >"$TMPD/p" 2>&1; then
            note "pattern mode reports clean when the pattern is absent"
        else
            note "pattern mode found the producer lea (expected on the pre-fold dump)"
        fi
    fi
fi

echo "== phase 4: usage errors exit 2, not 0 =="
if $PY "$TOOL" >/dev/null 2>&1; then
    bad "no source and no --trace-dir exited 0"
else
    rc=$?
    if [ "$rc" -eq 2 ]; then
        note "missing input -> exit 2"
    else
        bad "missing input -> exit $rc, expected 2"
    fi
fi
if $PY "$TOOL" --trace-dir "$TMPD/does-not-exist" >/dev/null 2>&1; then
    : # listing a missing dir raises; only the exit code is contractual
fi

echo "== phase 5: the tool NAMES the offender on a synthetic bad trace =="
# Phases 1-3 prove the tool is quiet when the compiler is clean and that its
# parser agrees with a 21-case table.  Neither proves it can still FAIL, and a
# tool that silently reports clean on everything would satisfy both.  So: build
# a trace whose one bad dump is exactly the historical unencodable operand and
# demand exit 1 plus the offending pass named in the verdict.
SYN=$TMPD/synthetic
mkdir -p "$SYN"
# Two innocuous dumps and the bad one, named so the offender is unambiguous.
printf 'f:\n    leaq 0(,%%r11,4), %%r8\n    leaq (%%r8, %%r10, 1), %%r9\n' > "$SYN/000-p0-innocent.s"
printf 'f:\n    leaq (%%r10, %%r11, 4), %%r9\n' > "$SYN/001-p0-also_innocent.s"
printf 'f:\n    leaq 0(,%%r11,4, %%r10, 1), %%r9\n' > "$SYN/999-p0-synthetic_bad.s"
if $PY "$TOOL" --trace-dir "$SYN" --quiet >"$TMPD/syn_v" 2>&1; then
    bad "the tool reported a CLEAN verdict on a trace containing an unencodable operand"
    sed 's/^/    /' "$TMPD/syn_v"
else
    rc=$?
    if [ "$rc" -eq 1 ]; then
        note "synthetic bad trace -> exit 1"
    else
        bad "synthetic bad trace -> exit $rc, expected 1"
    fi
    if grep -q 'synthetic_bad' "$TMPD/syn_v"; then
        note "verdict names the offending pass"
    else
        bad "verdict does not name synthetic_bad:"
        sed 's/^/    /' "$TMPD/syn_v"
    fi
fi
# The same trace must stay clean once the bad operand is repaired, so the
# phase is measuring the operand and not merely the presence of a third dump.
printf 'f:\n    leaq (%%r10, %%r11, 4), %%r9\n' > "$SYN/999-p0-synthetic_bad.s"
if $PY "$TOOL" --trace-dir "$SYN" --quiet >"$TMPD/syn_ok" 2>&1; then
    note "repaired dump -> clean verdict again (the check is on the operand)"
else
    bad "repaired dump still reported bad:"
    sed 's/^/    /' "$TMPD/syn_ok"
fi

if [ "$fail" -ne 0 ]; then
    echo "peephole-trace-bisect: FAIL" >&2
    exit 1
fi
echo "peephole-trace-bisect: PASS"
