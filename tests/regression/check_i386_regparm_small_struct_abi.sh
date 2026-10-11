#!/usr/bin/env bash
# Regparm small-struct gate: a 1-3 byte struct that is a subobject of a larger
# by-value argument must survive every i386 -mregparm convention, and it now
# does.  The class was found while answering "is the aggregate over-read fix
# complete?" (round 13), back when the answer was no:
#
#   struct S { unsigned char b[1]; };
#   struct T { unsigned long long pad; struct S s; };
#   eat(s) + eat(t.s)          /* eat: noinline, folds the 1 byte it was given */
#
# `eat(t.s)` must return 8 in every ABI, because that is the byte, and gcc
# returns 16 for the sum.  The regression was in the i686 call-site staging: the
# parked subobject pointer was reloaded into %eax *after* the loop had written
# the word whose destination is %eax, so the first regparm register received the
# source address instead of the byte -- a different wrong answer per
# configuration, and not even stable between runs of one binary, which is what
# reading memory the program never wrote looks like.  Reloading the parked
# pointer only while the descending word loop continues fixed it: all six
# suspect configurations now agree with gcc deterministically (re-measured on
# CI: 0 divergent, 12 executed, 0 skipped).  -m64 agreed throughout, including
# while the bug was live, so the x86-64 backend was never in this class; the
# shared *classifier* was fixed in round 12 and the regparm staging path was the
# other consumer of it.
#
# The oracle is gcc plus the program's own text: the value of a 1-byte object is
# not ABI-dependent, so agreement is a fact about the emitted code rather than a
# preference for another compiler's convention.
#
# Budget semantics: KNOWN is the measured count of divergent suspect
# configurations, latched exactly in both directions -- MORE is a regression,
# FEWER means the measurement moved and must be written down.  The debt is
# repaid, so it defaults to 0.  A "<= KNOWN" latch was rejected from the start
# because a gate that accepts "better" has already cost this project, and the
# vacuity arms below are what keep "0 divergences" from meaning "measured
# nothing".
set -uo pipefail
cd "$(dirname "$0")/../.." || exit 2

LCCC=${LCCC:-target/fastbuild/lccc}
# Re-measured after the staging fix: 0 of 6 suspect configurations diverge.
# Raise this only together with a new measurement.
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

# Divergence candidates: the i386 register-argument conventions.
declare -a SUSPECT=(
    "-m32 -mregparm=1 -O0"
    "-m32 -mregparm=1 -O2"
    "-m32 -mregparm=2 -O0"
    "-m32 -mregparm=2 -O2"
    "-m32 -mregparm=3 -O0"
    "-m32 -mregparm=3 -O2"
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

gcc_opts_for() {
    # gcc rejects `-mcmodel=kernel` with PIC and has no `-freg-struct-return`
    # problem; nothing here needs filtering today, and an empty filter is
    # written down rather than assumed.
    printf '%s' "$1"
}

div=0; ctrl_div=0; executed=0; skipped=0
sus_exec=0; sus_skip=0; ctrl_exec=0; ctrl_skip=0
report() { printf '  %-34s lccc=%-4s gcc=%-4s %s\n' "$1" "$2" "$3" "$4"; }

for cfg in "${SUSPECT[@]}" "${CONTROLS[@]}"; do
    case " ${SUSPECT[*]} " in
        *" $cfg "*) kind=suspect ;;
        *) kind=control ;;
    esac
    gflag=$(gcc_opts_for "$cfg")
    if ! gcc $gflag "$TMP/p2.c" -o "$TMP/g.exe" 2>"$TMP/g.err"; then
        skipped=$((skipped + 1))
        if [ "$kind" = suspect ]; then sus_skip=$((sus_skip + 1)); else ctrl_skip=$((ctrl_skip + 1)); fi
        continue
    fi
    "$TMP/g.exe"; want=$?
    if ! $LCCC $cfg "$TMP/p2.c" -o "$TMP/l.exe" 2>"$TMP/l.err"; then
        head -2 "$TMP/l.err"
        div=$((div + 1))
        report "$cfg" "FAIL" "$want" "**DIVERGE (compile)**"
        continue
    fi
    executed=$((executed + 1))
    if [ "$kind" = suspect ]; then sus_exec=$((sus_exec + 1)); else ctrl_exec=$((ctrl_exec + 1)); fi
    "$TMP/l.exe"; got=$?
    if [ "$got" != "$want" ]; then
        if [ "$kind" = suspect ]; then div=$((div + 1)); else ctrl_div=$((ctrl_div + 1)); fi
        report "$cfg" "$got" "$want" "**DIVERGE** ($kind)"
    fi
done

echo "small-struct regparm ABI: $div divergent suspect configuration(s) " \
     "(budget $KNOWN), $ctrl_div control divergence(s) (budget 0), " \
     "$executed executed, $skipped skipped because gcc rejected the flags"

rc=0
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
# licence to believe the class is clean.  Either every suspect configuration ran
# (compare against the budget), or none could (say so, and keep the controls
# honest), or the harness is looking at a half-populated host -- which is a
# failure, since a partial count matches neither a clean pass nor a measured one.
if [ "$sus_exec" -eq 0 ]; then
    echo "SKIP: no i386 configuration was executable here (32-bit toolchain absent);" \
         " $ctrl_exec control configuration(s) agreed, class unmeasured on this host"
    if [ "$ctrl_exec" -eq 0 ]; then
        echo "FAIL: not even a control ran -- the harness is broken, not the toolchain"
        exit 1
    fi
    exit 0
fi
if [ "$sus_exec" -ne "${#SUSPECT[@]}" ]; then
    echo "FAIL: $sus_exec of ${#SUSPECT[@]} suspect configurations ran -- a partial sweep" \
         " matches neither a clean pass nor a host that cannot run the class"
    exit 1
fi
if [ "$ctrl_div" -ne 0 ]; then
    echo "FAIL: a control configuration diverges -- the class has widened beyond i386 -mregparm"
    rc=1
fi
if [ "$div" -gt "$KNOWN" ]; then
    echo "FAIL: $div > $KNOWN -- the class is back: a small subobject aggregate is mis-passed under -mregparm again"
    rc=1
elif [ "$div" -lt "$KNOWN" ]; then
    echo "FAIL: $div < $KNOWN -- fewer divergences than the latch records; re-measure and lower the budget"
    rc=1
fi
[ "$rc" -eq 0 ] && echo "check_i386_regparm_small_struct_abi: PASS ($div == $KNOWN divergences, class exercised)"
exit $rc
