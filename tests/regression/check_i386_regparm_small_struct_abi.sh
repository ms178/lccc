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
# Attribution note: "lccc is wrong" rests on gcc + the program's own text (the
# value of a 1-byte object is not ABI-dependent). The mechanism is NOT
# established -- the leading suspect is the call-site aggregate copy using the
# parameter slot's address instead of the subobject's, which would explain both
# the offset and the garbage -- and a fix must not be written from this comment.
#
# Budget semantics: == KNOWN passes; MORE fails as a regression; FEWER fails as
# "debt repaid, delete this gate".  An exact-count latch was chosen over
# "<= KNOWN" because a partial fix here (say regparm=1 only) would otherwise be
# invisible, and this project has already paid for gates that accept "better".
set -uo pipefail
cd "$(dirname "$0")/../.." || exit 2

LCCC=${LCCC:-target/fastbuild/lccc}
KNOWN=${KNOWN:-6}
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
if [ "$sus_exec" -ne "${#SUSPECT[@]}" ]; then
    echo "FAIL: $sus_exec of ${#SUSPECT[@]} suspect configurations ran -- a partial sweep" \
         " matches neither a fixed pass nor the documented debt"
    exit 1
fi
if [ "$ctrl_div" -ne 0 ]; then
    echo "FAIL: a control configuration diverges -- the class has widened beyond i386 -mregparm"
    rc=1
fi
if [ "$div" -gt "$KNOWN" ]; then
    echo "FAIL: $div > $KNOWN -- more mis-passed configurations than documented"
    rc=1
elif [ "$div" -lt "$KNOWN" ]; then
    echo "FAIL: $div < $KNOWN -- some of this debt is repaid; re-measure and lower the budget, or delete this gate at 0"
    rc=1
fi
[ "$rc" -eq 0 ] && echo "check_i386_regparm_small_struct_abi: PASS (documented debt, exactly as counted)"
exit $rc
