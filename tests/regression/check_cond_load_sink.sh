#!/usr/bin/env bash
# SV-03 conditional-load sinking: executable differential acceptance gate.
#
# WHY THIS EXISTS
# ---------------
# `if_convert` used to refuse every `cond ? *p : *q` diamond because hoisting
# the arms would speculate a LOAD that can trap.  SV-03 instead speculates only
# the pure ADDRESS chain, emits one `Select { ty: Ptr }` and keeps exactly one
# Load.  An IR-only unit test cannot prove the crucial safety property: the
# untaken pointer is a PROT_NONE page here, so ANY speculated load SIGSEGVs.
#
# This gate is mutation-verified: with CCC_NO_COND_LOAD_SINK=1 the emitted code
# MUST revert to the branchy two-load diamond.  A green gate whose OFF state is
# already branchless proves nothing.  `--self-test` injects synthetic assembly
# into the same predicates so a weakened checker fails loudly.
#
# Toolchain comparison: GCC 16.2 / Clang 23.1.5 / LCCC ON / LCCC OFF must all
# print the identical checksum; GCC and Clang are the reference oracles, not
# the implementation under test.  Missing oracles are reported, never faked.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
GCC16=${GCC16:-gcc-16}
CLANG23=${CLANG23:-clang-23}
# Optional fourth oracle.  The hosted icx-oracle job passes its verified
# ICX 2026.2 binary here; locally Intel's APT repo is unreachable, so this
# leg is reported as a SKIP rather than faked with a surrogate compiler.
ICX=${ICX:-icx}
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
src=$root/tests/regression/cond_load_sink.c
tmp=${TMPDIR:-/tmp}/lccc-cond-load-sink.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
expect='-13.50'

# ── pure predicates (exercised by --self-test) ────────────────────────────
# Extract the body of one LCCC-emitted function from a .s file.
fn_body() {
    sed -n "/^$1:/,/^\\.size $1,/p" "$2"
}

# ON shape: one pointer cmov, one scalar load, no control flow.
on_shape_ok() {
    local body=$1
    [[ $(printf '%s\n' "$body" | grep -c 'cmov') -ge 1 ]] || return 1
    [[ $(printf '%s\n' "$body" | grep -c 'movss') -eq 1 ]] || return 1
    [[ $(printf '%s\n' "$body" | grep -cE '^[[:space:]]*j[a-z]+[[:space:]]') -eq 0 ]] || return 1
}

# OFF shape: no cmov, exactly two arm loads and at least one branch.
off_shape_ok() {
    local body=$1
    [[ $(printf '%s\n' "$body" | grep -c 'cmov') -eq 0 ]] || return 1
    [[ $(printf '%s\n' "$body" | grep -c 'movss') -eq 2 ]] || return 1
    [[ $(printf '%s\n' "$body" | grep -cE '^[[:space:]]*j[a-z]+[[:space:]]') -ge 1 ]] || return 1
}

if [[ ${1:-} == --self-test ]]; then
    good_on=$'pick:\n    testl %edx, %edx\n    cmovneq %rdi, %rsi\n    movss (%rsi), %xmm0\n    ret\n.size pick, .-pick'
    good_off=$'pick:\n    testl %edx, %edx\n    je .LBB2\n    movss (%rdi), %xmm2\n    jmp .LBB3\n    movss (%rsi), %xmm2\n    ret\n.size pick, .-pick'
    on_shape_ok "$good_on"  || { echo 'self-test: valid ON shape rejected'; exit 1; }
    off_shape_ok "$good_off" || { echo 'self-test: valid OFF shape rejected'; exit 1; }
    # Each mutation must be REJECTED by at least one predicate.
    ! on_shape_ok "${good_on/movss/movaps}" || { echo 'self-test: ON without a load accepted'; exit 1; }
    ! on_shape_ok "$good_off" || { echo 'self-test: branchy OFF accepted as ON'; exit 1; }
    ! off_shape_ok "$good_on" || { echo 'self-test: cmov ON accepted as OFF'; exit 1; }
    ! off_shape_ok "${good_off/movss/movaps}" || { echo 'self-test: OFF with one load accepted'; exit 1; }
    echo 'PASS cond-load-sink self-test (5 synthetic shape mutations rejected)'
    exit 0
fi

[[ -x $CCC || -f $CCC ]] || { echo "FAIL: LCCC driver not found: $CCC"; exit 1; }

compile_run() {  # compile_run <label> <compiler> [env assignments...]
    local label=$1 cc=$2; shift 2
    local out="$tmp/$label.out" log="$tmp/$label.log"
    if ! env "$@" "$cc" -O2 -Wall -Wextra -Werror "$src" -o "$tmp/$label.bin" >"$log" 2>&1; then
        echo "FAIL: $label compile"; cat "$log"; return 1
    fi
    if ! "$tmp/$label.bin" >"$out" 2>"$log"; then
        echo "FAIL: $label execution (probably a speculated load faulted)"; cat "$log"; return 1
    fi
    if [[ $(cat "$out") != "$expect" ]]; then
        echo "FAIL: $label stdout $(cat "$out") != $expect"; return 1
    fi
    echo "PASS $label stdout=$expect"
}

compile_run lccc_on  "$CCC"
compile_run lccc_off "$CCC" CCC_NO_COND_LOAD_SINK=1
if command -v "$GCC16" >/dev/null 2>&1; then
    compile_run gcc16 "$GCC16"
else
    echo "SKIP gcc16 ($GCC16 unavailable)"
fi
if command -v "$CLANG23" >/dev/null 2>&1; then
    compile_run clang23 "$CLANG23"
else
    echo "SKIP clang23 ($CLANG23 unavailable)"
fi
if command -v "$ICX" >/dev/null 2>&1; then
    compile_run icx2026 "$ICX"
else
    echo "SKIP icx2026 ($ICX unavailable; local sandbox cannot reach Intel PPA)"
fi

# The actual code-quality contract: all three shapes collapse to a single
# load on ON and remain a two-load diamond on OFF.
for mode in on off; do
    if [[ $mode == off ]]; then extra=(CCC_NO_COND_LOAD_SINK=1); else extra=(); fi
    env "${extra[@]}" "$CCC" -O2 -S "$src" -o "$tmp/$mode.s"
    for fn in pick pick_idx pick_chain; do
        body=$(fn_body "$fn" "$tmp/$mode.s")
        [[ -n $body ]] || { echo "FAIL: $mode asm has no $fn body"; exit 1; }
        if [[ $mode == on ]]; then
            if ! on_shape_ok "$body"; then
                echo "FAIL: ON $fn is not a pointer-select + one load"; printf '%s\n' "$body"; exit 1
            fi
        elif ! off_shape_ok "$body"; then
            echo "FAIL: OFF $fn is not the branchy two-load diamond"; printf '%s\n' "$body"; exit 1
        fi
        echo "PASS $mode $fn shape"
    done
done

# -O0 must remain correct even when the optimizer is not expected to fire.
if ! CCC_NO_COND_LOAD_SINK=1 "$CCC" -O0 -Wall -Wextra -Werror "$src" -o "$tmp/o0.bin"; then
    echo 'FAIL: -O0 compile'; exit 1
fi
[[ $("$tmp/o0.bin") == "$expect" ]] || { echo "FAIL: -O0 stdout"; exit 1; }
echo "PASS -O0 stdout=$expect"
echo 'PASS cond-load-sink differential gate'
