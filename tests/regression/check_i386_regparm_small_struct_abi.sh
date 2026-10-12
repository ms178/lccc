#!/usr/bin/env bash
# Documented debt, gated: a by-value subobject struct smaller than 4 bytes is
# mis-passed under -mregparm (i386), and silently so.
#
# Found while answering "is the aggregate over-read fix complete?" (round 13).
# The answer is no, and this file is why the question stays answered: the class
# is reproduced, attributed to a call convention, and counted.
#
#   struct S { unsigned char b[1]; };
#   struct T { unsigned long long pad; struct S s; };
#   eat(s) + eat(t.s)          /* eat: noinline, folds the 1 byte it was given */
#
# `eat(t.s)` must return 8 in every ABI, because that is the byte. gcc returns
# 16 for the sum. lccc returns 16 with -mregparm=0 and the i386 default, and
# 72 / 232 / 40 / 8 for -mregparm=2,3 at -O0,-O2 -- a different wrong answer per
# configuration, which is what reading past the end of the one-byte member looks
# like. -m64 agrees everywhere (measured: 10 configurations, all ok), so the
# x86-64 backend is not in this class; the shared *classifier* was the thing
# fixed in round 12, and the regparm staging path is a different consumer of it.
#
# Attribution note: "lccc is wrong" rested on gcc + the program's own text (the
# value of a 1-byte object is not ABI-dependent). The mechanism was never
# established here -- the leading suspect was the call-site aggregate copy using
# the parameter slot's address instead of the subobject's, which would explain
# both the offset and the garbage -- and no fix was written from this comment.
#
# Status, measured on this tree with a real 32-bit toolchain (`gcc-multilib`,
# 2026-10-12): the *wrong-answer* face of the class is gone.  All six suspect
# configurations and all six controls return gcc's answer for the program above,
# and the whole class of 72/232/40/8 values that the previous host recorded is
# not reproducible.  So `KNOWN` is 0 and `EXPECT` pins `agree`: this file is a
# latch on a closed defect now, not a debt record, and it stays because the
# defect was silent.
#
# A number that agrees is only worth what the program can see, so the gate also
# measures its own sensitivity (`p2s.c`, below): the same object read one byte
# later must produce a *different* answer.  On the 0xAA-poisoned fixture the
# correct byte is worth 16 and the poisoned byte is worth 178 = 8 + 0xAA, so an
# agreement at 16 cannot be a wrong-byte read that happened to land on 8.
# Without that pair, "everyone returns 16" would be indistinguishable from
# "everyone reads the same padding and the padding is zero".
#
# What this gate still cannot see, stated so nobody reads it as coverage: an
# over-read whose value is *ignored* -- reading 4 bytes and returning the first
# is a memory-safety bug with a correct answer, and no exit status can tell.
# That face belongs to assembly-level assertions on the staging sequence
# (`-m32 -S`, `tests/regression/check_i386_call_staging_asm.sh`), not here.
#
# Budget semantics: == KNOWN passes; MORE fails as a regression; FEWER fails as
# "debt repaid, delete this gate".  An exact-count latch was chosen over
# "<= KNOWN" because a partial fix here (say regparm=1 only) would otherwise be
# invisible, and this project has already paid for gates that accept "better".
# At KNOWN=0 the "fewer" arm is unreachable by arithmetic, which is the point:
# any divergence from the oracle is now a regression.
set -uo pipefail
cd "$(dirname "$0")/../.." || exit 2

LCCC=${LCCC:-target/fastbuild/lccc}
# A missing artifact is a usage error, not a measurement: without this the gate reported
# "2 configuration(s) could not be compiled by lccc" on a box that had never built one,
# which is the same misattribution this file exists to remove.
[[ -x "$LCCC" ]] || { echo "FAIL: no compiler to measure at $LCCC (build it, or set LCCC=...)" >&2; exit 2; }
CC=${CC:-gcc}   # an overridable oracle, so the self-test below can drive it
# 0 since the 2026-10-12 multilib re-measurement closed the class; the six
# values this held (72/232/40/8 on four configurations, unrecorded on two) are
# superseded, and re-pinning a nonzero number here requires a measurement, not
# a wish that the gate had something to say about.
KNOWN=${KNOWN:-0}
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

cat > "$TMP/p2.c" <<'C'
#include <stdint.h>
struct S { unsigned char b[1]; };
struct T { unsigned long long pad; struct S s; };
__attribute__((noinline)) static unsigned eat(struct S s) {
    unsigned h = 0;
    for (int i = 0; i < 1; i++) h = h * 131u + s.b[i];
    return h;
}
int main(void) {
    struct S s; s.b[0] = 8;
    struct T t; t.pad = 0xdeadbeefULL; t.s = s;
    return (int)((eat(s) + eat(t.s)) % 251);
}
C

# The same program with the enclosing object poisoned, so that a byte the
# subobject does not own cannot be mistaken for the byte it does: `struct T`'s
# tail padding (offsets 9..15) becomes 0xAA instead of whatever the frame left
# there.  This is the fixture the 72/232/40/8 values could not be trusted on --
# they were garbage, and garbage can come out looking right.
cat > "$TMP/p2p.c" <<'C'
#include <stdint.h>
struct S { unsigned char b[1]; };
struct T { unsigned long long pad; struct S s; };
__attribute__((noinline)) static unsigned eat(struct S s) {
    unsigned h = 0;
    for (int i = 0; i < 1; i++) h = h * 131u + s.b[i];
    return h;
}
int main(void) {
    union { struct T t; unsigned char raw[sizeof(struct T)]; } u;
    for (unsigned i = 0; i < sizeof u; i++) u.raw[i] = 0xAA;
    u.t.pad = 0;
    u.t.s.b[0] = 8;
    struct S s; s.b[0] = 8;
    return (int)((eat(s) + eat(u.t.s)) % 251);
}
C

# The sensitivity control: the *same* object, the same call, one byte further in.
# `u.raw[9]` is inside the union, so this is not undefined behaviour -- it is a
# well-defined read of the poison.  It must answer 178 where the real subobject
# answers 16, in both compilers; if it ever answers 16 as well, this fixture has
# stopped being able to tell a wrong-byte read from a right one, and a green
# sweep below would mean nothing.
sed 's/eat(u\.t\.s)/eat(*(const struct S *)((const char *)\&u + 9))/' \
    "$TMP/p2p.c" > "$TMP/p2s.c"
grep -q 'u + 9' "$TMP/p2s.c" || { echo "FAIL: the sensitivity control did not get written"; exit 2; }

# Divergence candidates: the i386 register-argument conventions.
declare -a SUSPECT=(
    "-m32 -mregparm=1 -O0"
    "-m32 -mregparm=1 -O2"
    "-m32 -mregparm=2 -O0"
    "-m32 -mregparm=2 -O2"
    "-m32 -mregparm=3 -O0"
    "-m32 -mregparm=3 -O2"
)
# Per-configuration expectation, in the order of SUSPECT above.  A total can be
# preserved by moving the bug: fix `-mregparm=1 -O0` while breaking
# `-mregparm=3 -O2` and `div` stays 6.  Pinning *which* configuration is wrong (and,
# where the multilib host recorded one, the value it returns) removes that degree of
# freedom, so the gate fails on a swap even when the count is unchanged.
#
#   diverge:72  documented debt, and the wrong value must be 72
#   diverge     documented debt, value not recorded (see the header: only four of the
#               six were written down, and inventing the other two would be a number
#               with no measurement behind it)
#   agree       must match the oracle
#
# These came from a host with a 32-bit toolchain.  On a host without one the whole
# suspect class is unmeasurable and the gate says so; the table is still exercised by
# --self-test, which drives it with a fake oracle, so the logic does not depend on
# having the multilib package installed.
declare -a EXPECT=(
    "agree"
    "agree"
    "agree"
    "agree"
    "agree"
    "agree"
)

# Controls: same program, same oracle, must agree.  If one of these starts to
# diverge the class has widened and that is a new defect, not this budget.
declare -a CONTROLS=(
    "-m32 -O0"
    "-m32 -O2"
    "-m32 -mregparm=0 -O2"
    "-m32 -freg-struct-return -O2"
    "-m64 -O0"
    "-m64 -O2"
)

# --- self-test -------------------------------------------------------------
# The budget below counts *wrong answers per configuration*, per program.  Four things
# used to be possible that this pins, each with a witness:
#   (a) a sweep in which every suspect configuration raised an internal error added up
#       to exactly the documented debt and printed PASS;
#   (b) no way to say "this compiler agrees everywhere" -- the pass shape had to be
#       constructed, so here it is: an all-agreeing run exits 0 and says so;
#   (c) a bug moved between configurations, invisible to any count that both the table
#       and the budget agree with; and now that the budget is 0, the *count* alone
#       could catch it, so this witness asserts the table named the configuration too;
#   (d) suspect configurations that fail to compile falling into the "toolchain absent,
#       SKIP" branch, which is how a broken compiler used to read as a host limitation.
# Plus the two arms the closed class introduced: (e) a control diverging (the class has
# widened), and (f) a stale nonzero budget, which has to fail rather than "pass" a
# repaid class.
#
# Every witness drives this whole script against a fake oracle and fake compilers, so
# they are meaningful on a host with no 32-bit toolchain -- which is the point, since
# the values this table used to carry came from a host that has one.
if [ "${1:-}" = "--self-test" ]; then
    tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
    # The fake oracle accepts any flag set, including -m32 this host cannot build, and
    # always exits 16 -- the ABI-correct answer (8 + 8 mod 251) -- except that it knows
    # the *poisoned-byte* program must answer 178 (8 + 0xAA), so the sensitivity arm of
    # a clean sweep runs instead of failing on the fakes.
    printf '%s\n' '#!/bin/sh' 'for a in "$@"; do case "$a" in -o) n=next; continue;; esac; if [ "$n" = next ]; then f="$a"; n=; fi; done' \
        'case "$*" in *p2s.c*) v=178 ;; *) v=16 ;; esac' \
        'printf "#!/bin/sh\\nexit %s\\n" "$v" > "$f"' 'chmod +x "$f"' 'exit 0' > "$tmp/gcc"
    # (a) every configuration fails to compile: same count, opposite meaning.
    printf '%s\n' '#!/bin/sh' 'echo "error: internal compiler error" >&2' 'exit 1' > "$tmp/ice"
    # (c) one suspect configuration returns the poisoned byte: the value a wrong
    # staging order produces, and the only difference between this compiler and the
    # oracle.  Both programs of that cell are hit, as they would be in a real sweep.
    printf '%s\n' '#!/bin/sh' 'for a in "$@"; do case "$a" in -o) n=next; continue;; esac; if [ "$n" = next ]; then f="$a"; n=; fi; done' \
        'case "$*" in *"-m32 -mregparm=3 -O2"*) v=99 ;; *p2s.c*) v=178 ;; *) v=16 ;; esac' \
        'printf "#!/bin/sh\\nexit %s\\n" "$v" > "$f"' 'chmod +x "$f"' 'exit 0' > "$tmp/one_cell_wrong"
    # (d) the audit's own scenario: the *suspect* class cannot be compiled by lccc
    # while the controls run -- which used to fall into "toolchain absent, SKIP".
    printf '%s\n' '#!/bin/sh' 'for a in "$@"; do case "$a" in -o) n=next; continue;; esac; if [ "$n" = next ]; then f="$a"; n=; fi; done' \
        'case "$*" in *regparm*) echo "error: internal compiler error" >&2; exit 1 ;; esac' \
        'printf "#!/bin/sh\\nexit 16\\n" > "$f"' 'chmod +x "$f"' 'exit 0' > "$tmp/ice_suspects"
    # (e) a control diverges: -m64 is not in this class, so a wrong answer there is a
    # new defect and must not be absorbed by a budget.
    printf '%s\n' '#!/bin/sh' 'for a in "$@"; do case "$a" in -o) n=next; continue;; esac; if [ "$n" = next ]; then f="$a"; n=; fi; done' \
        'case "$*" in *"-m64 -O0"*) v=7 ;; *p2s.c*) v=178 ;; *) v=16 ;; esac' \
        'printf "#!/bin/sh\\nexit %s\\n" "$v" > "$f"' 'chmod +x "$f"' 'exit 0' > "$tmp/control_off"
    chmod +x "$tmp"/*
    fails=0
    run_witness() { # $1 = fake compiler, $2 = KNOWN
        ABI_GATE_SELFCHECK=done LCCC="$tmp/$1" CC="$tmp/gcc" KNOWN="$2" \
            bash "${BASH_SOURCE[0]}" 2>&1
    }
    # Every witness also has to sweep the whole grid: 24 cells (12 configurations x 2
    # programs) plus the 3 sensitivity compiles.  A witness that ran less is not a
    # witness, and "24 cells agree" is only worth reading if 24 were attempted.
    want_executed=27
    # Each token is matched on its own.  A single chained glob over the whole
    # transcript would depend on the order the script happens to print things in,
    # and an assertion that fails because a line moved is an assertion that gets
    # deleted rather than fixed.
    has() { case "$1" in *"$2"*) return 0 ;; esac
        printf "  FAIL: %s: no line containing: %s\n" "$3" "$2"
        printf "%s\n" "$1" | tail -6
        return 1; }

    out=$(run_witness ice 0); rc=$?
    case "$out" in
        *"could not be compiled by lccc"*)
            echo "  ok: (a) an all-ICE sweep is a FAIL, not a satisfied budget" ;;
        *)
            echo "  FAIL: (a) an all-ICE sweep exited $rc saying: $(printf '%s' "$out" | tail -1)"
            fails=$((fails + 1)) ;;
    esac

    out=$(run_witness gcc 0); rc=$?
    if has "$out" "$want_executed executed" "(b)" && has "$out" "closed class held" "(b)"; then
        echo "  ok: (b) an all-agreeing sweep is the pass shape, and says which cells it swept"
    else
        fails=$((fails + 1))
    fi
    if [ "$rc" -ne 0 ]; then
        echo "  FAIL: (b) exited $rc"
        printf '%s\n' "$out" | tail -4
        fails=$((fails + 1))
    fi

    out=$(run_witness one_cell_wrong 0); rc=$?
    if has "$out" "expected agree -- a new defect" "(c)" && has "$out" "$want_executed executed" "(c)" \
       && has "$out" "2 > 0 -- more mis-passed" "(c)"; then
        echo "  ok: (c) one wrong configuration is named, counted, and not absorbed"
    else
        fails=$((fails + 1))
    fi

    out=$(run_witness ice_suspects 0); rc=$?
    case "$out" in
        *"could not be compiled by lccc"*)
            echo "  ok: (d) suspect configurations that fail to compile never reach the SKIP branch" ;;
        *)
            echo "  FAIL: (d) expected the compile-failure verdict, got: $(printf '%s' "$out" | tail -1)"
            fails=$((fails + 1)) ;;
    esac

    out=$(run_witness control_off 0); rc=$?
    if has "$out" "class has widened" "(e)" && has "$out" "$want_executed executed" "(e)"; then
        echo "  ok: (e) a control that diverges is a new defect, not this budget"
    else
        fails=$((fails + 1))
    fi

    # (f) the budget this file carried before the class closed.  With an agreeing
    # compiler it has to fail as "debt repaid", because a latch that only fires on
    # *more* wrong answers is a latch that celebrates nothing happening.
    out=$(run_witness gcc 6); rc=$?
    case "$out" in
        *"0 < 6"*"debt is repaid"*)
            echo "  ok: (f) a stale nonzero budget fails instead of passing a repaid class" ;;
        *)
            echo "  FAIL: (f) KNOWN=6 against an agreeing compiler read as: $(printf '%s' "$out" | tail -1)"
            fails=$((fails + 1)) ;;
    esac
    if [ "$rc" -eq 0 ]; then
        echo "  FAIL: (f) exited 0 -- the under-budget arm is unreachable"
        fails=$((fails + 1))
    fi

    if [ "$fails" -ne 0 ]; then
        echo "self-test: $fails witness(es) failed"
        exit 1
    fi
    echo "self-test: ok"
    exit 0
fi

if [ "${1:-}" != "--self-test" ] && [ "${ABI_GATE_SELFCHECK:-}" != "done" ]; then
    ABI_GATE_SELFCHECK=done bash "${BASH_SOURCE[0]}" --self-test || exit 1
fi

gcc_opts_for() {
    # gcc rejects `-mcmodel=kernel` with PIC and has no `-freg-struct-return`
    # problem; nothing here needs filtering today, and an empty filter is
    # written down rather than assumed.
    printf '%s' "$1"
}

div=0; ctrl_div=0; executed=0; skipped=0; compile_fail=0
# The three ways the per-configuration table can be violated, counted separately from
# the ordinary divergence tally so each gets its own sentence below.
repaid=0; shifted=0; fixed=0
# `differ` is the count the old latch used: suspect configurations where lccc and gcc
# disagree, whatever the table says about why.  Printing it lets witness (c) show that
# a total-preserving swap really does preserve the total the table replaced.
differ=0
sus_exec=0; sus_skip=0; ctrl_exec=0; ctrl_skip=0
PROG=p2p   # set by the sweep below; `report` prefixes it so a line names its program
report() { printf '  %-40s lccc=%-4s gcc=%-4s %s\n' "$PROG:$1" "$2" "$3" "$4"; }

expect_for() {
    # The expectation of one suspect configuration, by exact match on its flag
    # string; controls are always `agree`.  An unset SUSPECT entry is a bug in this
    # file, so it fails rather than defaulting to something permissive.
    local want
    want="${EXPECT[$1]:-}"
    if [[ -z $want ]]; then
        echo "FAIL: EXPECT has no entry for configuration #$1 (\"$2\") -- a suspect" \
             "configuration without a pinned outcome is how the count-only latch" \
             "survived in the first place" >&2
        exit 1
    fi
    printf '%s' "$want"
}

NPROG=2
# Two programs, one measurement grid: `p2` is the historical shape the debt was
# recorded on, `p2p` the poisoned one that makes a right answer mean something.
# Both are run so a reader can see the two agree with each other as well as with
# the oracle -- and so the day the poison starts to matter again, the two rows
# separate and say so.
for prog in p2 p2p; do
PROG=$prog
index=-1   # EXPECT is indexed per configuration, so it restarts for every program
for cfg in "${SUSPECT[@]}" "${CONTROLS[@]}"; do
    index=$((index + 1))
    case " ${SUSPECT[*]} " in
        *" $cfg "*) kind=suspect ;;
        *) kind=control ;;
    esac
    gflag=$(gcc_opts_for "$cfg")
    if ! "$CC" $gflag "$TMP/$prog.c" -o "$TMP/g.exe" 2>"$TMP/g.err"; then
        skipped=$((skipped + 1))
        if [ "$kind" = suspect ]; then sus_skip=$((sus_skip + 1)); else ctrl_skip=$((ctrl_skip + 1)); fi
        continue
    fi
    "$TMP/g.exe"; want=$?
    if ! $LCCC $cfg "$TMP/$prog.c" -o "$TMP/l.exe" 2>"$TMP/l.err"; then
        head -2 "$TMP/l.err"
        compile_fail=$((compile_fail + 1))
        report "$cfg" "FAIL" "$want" "**COMPILE FAILURE**"
        continue
    fi
    executed=$((executed + 1))
    if [ "$kind" = suspect ]; then sus_exec=$((sus_exec + 1)); else ctrl_exec=$((ctrl_exec + 1)); fi
    "$TMP/l.exe"; got=$?
    if [ "$kind" = control ]; then
        if [ "$got" != "$want" ]; then
            ctrl_div=$((ctrl_div + 1))
            report "$cfg" "$got" "$want" "**DIVERGE** (control)"
        fi
        continue
    fi
    if [ "$got" != "$want" ]; then differ=$((differ + 1)); fi
    expect=$(expect_for "$index" "$cfg") || exit 1
    case "$expect" in
        agree)
            # The debt on this configuration is gone: that is a re-measurement, not
            # an improvement to be quietly absorbed.
            if [ "$got" != "$want" ]; then
                div=$((div + 1))
                report "$cfg" "$got" "$want" "**DIVERGE** (expected agree -- a new defect)"
            else
                fixed=$((fixed + 1))
                report "$cfg" "$got" "$want" "ok (pinned to agree)"
            fi ;;
        diverge:*)
            want_bad=${expect#diverge:}
            if [ "$got" = "$want" ]; then
                report "$cfg" "$got" "$want" "**NOW AGREES** -- debt repaid on this" \
" configuration; re-measure and update EXPECT (a count that stays at $KNOWN by moving" \
" the bug elsewhere is not a pass)"
                repaid=$((repaid + 1))
            elif [ "$got" != "$want_bad" ]; then
                report "$cfg" "$got" "$want_bad" "**WRONG WRONG VALUE** -- documented as" \
" $want_bad, measured $got; the class moved, so update EXPECT with a fresh measurement"
                shifted=$((shifted + 1))
            else
                div=$((div + 1))
                report "$cfg" "$got" "$want" "known debt (value as documented)"
            fi ;;
        diverge)
            if [ "$got" = "$want" ]; then
                report "$cfg" "$got" "$want" "**NOW AGREES** -- debt repaid on this" \
" configuration; re-measure and update EXPECT"
                repaid=$((repaid + 1))
            else
                div=$((div + 1))
                observed_value="$got"
                report "$cfg" "$got" "$want" "known debt (value not pinned)"
            fi ;;
    esac
done
done

# --- the instrument, measured ---------------------------------------------------
#
# "Every configuration agrees with gcc" is worth exactly what the program can see.
# So the gate checks the fixture's own sensitivity before it is believed: the same
# call, handed the byte *one past* the subobject, has to produce a different answer.
# If it stops doing so -- because a libc changed the frame layout, because `eat` got
# inlined into a constant, because the poison loop was optimised away by a compiler
# that decided the read was out of lifetime -- then every agreement printed above
# would be measuring nothing, and the sweep would still exit 0.
#
# The shifted read is in bounds of the union, so it is well defined rather than
# merely observed: 8 + 0xAA is 178, and 8 + 8 is 16, in either compiler, at
# -m32 -mregparm=3 -O2.
sens_cfg="-m32 -mregparm=3 -O2"
sens_fail=0
sens_cells=0
if "$CC" $sens_cfg "$TMP/p2p.c" -o "$TMP/p.exe" 2>/dev/null; then
    # The arm is an i386 measurement like the sweep above, so a host without a
    # 32-bit toolchain does not get to report it as a failure -- and must not get
    # to report it as a pass either, hence the count in the summary line.
    "$TMP/p.exe"; base=$?
    "$CC" $sens_cfg "$TMP/p2s.c" -o "$TMP/s.exe" 2>/dev/null && "$TMP/s.exe"
    shift_v=$?
    "$LCCC" $sens_cfg "$TMP/p2s.c" -o "$TMP/sl.exe" 2>/dev/null && "$TMP/sl.exe"
    ours=$?
    sens_cells=3
    executed=$((executed + sens_cells))
    if [ "$base" = "$shift_v" ]; then
        echo "FAIL: the poison does not change the answer (both $base) -- the fixture can no" \
             " longer tell a wrong-byte read from a right one, so a green sweep above means nothing"
        sens_fail=1
    elif [ "$ours" != "$shift_v" ]; then
        echo "FAIL: lccc and gcc disagree on the *shifted* program ($ours vs $shift_v) -- the" \
             " fixture reads differently per compiler, so its agreement on the real call proves nothing"
        sens_fail=1
    else
        PROG=sens   # so the line reads `sens:...` rather than inheriting the last swept program
        report "$sens_cfg" "$ours" "$shift_v" "sensitivity ok (poisoned byte reads as $shift_v, subobject as $base)"
    fi
else
    echo "note: sensitivity arm not runnable here -- the oracle cannot build $sens_cfg"
fi
echo "small-struct regparm ABI:
 $div divergent suspect configuration(s) " \
     "(budget $KNOWN, $differ configuration(s) disagreed with gcc in total), " \
     "$compile_fail compile failure(s) (budget 0), " \
     "$ctrl_div control divergence(s) (budget 0), " \
     "$executed executed, $skipped skipped because gcc rejected the flags, " \
     "sensitivity arm: $sens_cells of 3 cells measured"

rc=0
[ "$sens_fail" -ne 0 ] && rc=1   # the instrument itself is the failure; see the line above
# Counted before the budget, and separately: a configuration lccc cannot compile is
# not one of the six wrong answers this file budgets.  Folding them together let a
# sweep in which the debt was fixed *and* the compiler broke print "documented debt,
# exactly as counted", and lets a new internal error hide inside an existing count.
if [ "$compile_fail" -ne 0 ]; then
    echo "FAIL: $compile_fail configuration(s) could not be compiled by lccc -- an internal error is a different claim from a wrong value, and a sweep of them would otherwise add up to exactly the documented debt"
    exit 1
fi
# The vacuity latch, applied to this gate itself: an enumeration that ran
# nothing prints a satisfied count and exits green without it.  A previous
# version of this sweep reported "588 cases, 0 divergences" while executing
# zero, because a missing #include made every case skip.
if [ "$executed" -eq 0 ]; then
    echo "FAIL: no configuration was executable -- the harness is broken"
    exit 1
fi
# All-or-nothing on the suspect class, because a hosted runner without a 32-bit
# toolchain cannot measure it at all, and "0 divergences" there would be a
# licence to believe the debt is gone.  Either every suspect configuration ran
# (compare against the budget), or none could (say so, and keep the controls
# honest), or the harness is looking at a half-populated host -- which is a
# failure, since a partial count matches neither a fixed bug nor a known one.
if [ "$sus_exec" -eq 0 ]; then
    echo "SKIP: no i386 configuration was executable here (32-bit toolchain absent);" \
         " $ctrl_exec control configuration(s) agreed, debt unmeasured on this host"
    if [ "$ctrl_exec" -eq 0 ]; then
        echo "FAIL: not even a control ran -- the harness is broken, not the toolchain"
        exit 1
    fi
    exit 0
fi
if [ "$sus_exec" -ne $(( ${#SUSPECT[@]} * NPROG )) ]; then
    echo "FAIL: $sus_exec of $(( ${#SUSPECT[@]} * NPROG )) suspect cells ran (configurations x programs) -- a partial sweep" \
         " matches neither a fixed pass nor the documented debt"
    exit 1
fi
if [ "$ctrl_div" -ne 0 ]; then
    echo "FAIL: a control configuration diverges -- the class has widened beyond i386 -mregparm"
    rc=1
fi
if [ "$repaid" -ne 0 ] || [ "$shifted" -ne 0 ]; then
    echo "FAIL: the per-configuration table no longer describes this tree" \
         "($repaid configuration(s) documented as debt now agree, $shifted disagree with" \
         "the recorded value) -- re-measure on a multilib host and update EXPECT." \
         "  This is the check the aggregate budget could not make: swapping which" \
         "  configuration is broken leaves \`div\` untouched."
    rc=1
fi
if [ "$div" -gt "$KNOWN" ]; then
    echo "FAIL: $div > $KNOWN -- more mis-passed configurations than documented"
    rc=1
elif [ "$div" -lt "$KNOWN" ]; then
    echo "FAIL: $div < $KNOWN -- some of this debt is repaid; re-measure and lower the budget, or delete this gate at 0"
    rc=1
fi
if [ "$rc" -eq 0 ] && [ "$KNOWN" -eq 0 ]; then
    echo "check_i386_regparm_small_struct_abi: PASS (closed class held: $(( ( ${#SUSPECT[@]} + ${#CONTROLS[@]} ) * NPROG )) cells agree with the oracle, sensitivity of the fixture verified, budget 0)"
elif [ "$rc" -eq 0 ]; then
    echo "check_i386_regparm_small_struct_abi: PASS (documented debt, exactly as counted)"
fi
exit $rc
