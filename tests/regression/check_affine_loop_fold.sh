#!/usr/bin/env bash
# ============================================================================
# check_affine_loop_fold.sh — AFFOLD: the affine exit-compare fold is a
# default-on pass of its own, exact at the type boundaries, and still refuses
# everything it cannot prove.
#
# Why this gate exists.  `canonicalise_affine_exit_cmps` used to run only on
# the clone produced by `loop_rotate`, which is opt-in AND refuses every nested
# loop (Guard E).  Every filter kernel's hot loop is the inner loop of a nest,
# so the fold -- and the `leaq 4(...)` per iteration it deletes -- was
# unreachable exactly where it pays: Callgrind on the `i + 4 < 4096` shape
# inside a nest measured 6 instructions/iteration against GCC's 5 (4.914G vs
# 4.094G Ir); after the fold it is 4.095G, i.e. parity.  The rewrite needs no
# CFG surgery, so it is now its own post-rotation phase, enabled by default at
# -O2+ and on the size pipelines, with `CCC_NO_AFFINE_EXIT_FOLD=1` as the kill
# switch.
#
# Contracts:
#
#   4. WRAPAROUND PROBES.  `wrap_affine_fold.c` / `wrap_loop_fold.c` are the
#      shapes where an unsigned add wraps and then feeds a signed compare --
#      the inputs a fold without the no-overflow licence gets wrong.  Each
#      probe carries its own reference implementation, must print `exact`, and
#      must print the SAME thing with the fold killed.
#
#   1. ORACLE PARITY.  `affine_loop_fold.c` reports the iteration count of
#      every kernel as well as its checksum (a wrong fold shows up first as a
#      wrong count), and its stdout must equal the GCC oracle's, byte for
#      byte, with the fold on, with it killed, with rotation on, with both
#      off, and at -O1 / -Os.
#
#   2. THE FOLD IS COUNTED AND IS NOT TIED TO ROTATION.  Under
#      `CCC_DEBUG_AFFINE_FOLD=1` the pass must report folds with no environment
#      at all and again with `CCC_LOOP_ROTATE=1`, and must report none under
#      its kill switch.
#
#   3. THE OBJECT CODE PROVES BOTH DIRECTIONS.  `selfloop` (one block, the
#      header is its own entry) and `while_guard` (guard-only) must fold too;
#      `nested_two` -- the nested
#      inner loop that only this pass can reach -- must compare the bare IV
#      against the folded bound (`cmpq $4092`) with no `leaq 4(...)` left;
#      `near_high` must contain the folded type-top bound LLONG_MAX - 4
#      (`movabsq $9223372036854775803`) and no offset materialisation, which is
#      the case a fold that skipped the representability check would get wrong;
#      and the refusals must still materialise their offsets (`refuse_rt_bound`
#      runtime bound, `refuse_rt_start` runtime seed).
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
GCC=${GCC:-gcc}
[[ -x $CCC ]] || { echo "check_affine_loop_fold: lccc not found at $CCC" >&2; exit 1; }
command -v "$GCC" >/dev/null 2>&1 || { echo "check_affine_loop_fold: no gcc oracle" >&2; exit 1; }

corpus=$here/affine_loop_fold.c
work=$(mktemp -d "${TMPDIR:-/tmp}/lccc-alfold.XXXXXX")
trap 'rm -rf "$work"' EXIT
fail=0
note() { printf '  %s\n' "$*"; }
bad() { printf '  FAIL: %s\n' "$*" >&2; fail=1; }

# ---------------------------------------------------------------- contract 1
note "contract 1: oracle parity across fold/rotation configurations"
"$GCC" -O2 -march=x86-64-v3 "$corpus" -o "$work/ref" || bad "gcc oracle failed to build"
expected=$(timeout 900 "$work/ref") || bad "gcc oracle exited non-zero"
case $expected in
    *"affine-loop-fold: all shapes exact"*) ;;
    *) bad "gcc oracle did not report the expected success line" ;;
esac

run_case() { # name  "flags"  "compiler env assignments"
    local name=$1 flags=$2 envs=$3
    local -a farr=() earr=()
    read -r -a farr <<< "$flags"
    read -r -a earr <<< "$envs"
    if ! env "${earr[@]}" "$CCC" "${farr[@]}" "$corpus" -o "$work/$name" > "$work/$name.build" 2>&1; then
        bad "$name: lccc failed to build"; return
    fi
    local out rc=0
    out=$(timeout 900 "$work/$name") || rc=$?
    [[ $rc -eq 0 ]] || { bad "$name: exited $rc (expected 0)"; return; }
    [[ $out == "$expected" ]] || bad "$name: stdout differs from the gcc oracle"
    [[ $out == "$expected" ]] && note "$name: parity"
}

run_case "fold-on"          "-O2 -march=x86-64-v3" ""
run_case "fold-killed"      "-O2 -march=x86-64-v3" "CCC_NO_AFFINE_EXIT_FOLD=1"
run_case "rot"              "-O2 -march=x86-64-v3" "CCC_LOOP_ROTATE=1"
run_case "rot-fold-killed"  "-O2 -march=x86-64-v3" "CCC_LOOP_ROTATE=1 CCC_NO_AFFINE_EXIT_FOLD=1"
run_case "O1"               "-O1 -march=x86-64-v3" ""
run_case "Os"               "-Os -march=x86-64-v3" ""
run_case "O3-rot"           "-O3 -march=x86-64-v3" "CCC_LOOP_ROTATE=1"

# ---------------------------------------------------------------- contract 2
note "contract 2: default-on reporting, independent of rotation, kill switch"
# The early unsigned normalizer has an independent disable switch and a
# distinct "(early unsigned)" suffix. Count only late/rotation fold reports.
count_reports() { # env...
    local out
    out=$(timeout 900 env "$@" "$CCC" -O2 -march=x86-64-v3 -S "$corpus" -o /dev/null 2>&1 >/dev/null) || {
        printf '%s\n' "$out" >&2; return 1;
    }
    printf '%s\n' "$out" | grep -Ec '^\[AFFOLD\] .*affine exit-compare folds: [1-9][0-9]*$' || true
}
n_def=$(count_reports CCC_DEBUG_AFFINE_FOLD=1)
n_rot=$(count_reports CCC_DEBUG_AFFINE_FOLD=1 CCC_LOOP_ROTATE=1)
n_kill=$(count_reports CCC_DEBUG_AFFINE_FOLD=1 CCC_NO_AFFINE_EXIT_FOLD=1)
note "folds reported: default=$n_def rotation=$n_rot killed=$n_kill"
[[ $n_def -gt 0 ]] || bad "the fold reported nothing by default (pass not in the pipeline?)"
[[ $n_rot -gt 0 ]] || bad "the fold reported nothing with rotation on"
[[ $n_kill -eq 0 ]] || bad "CCC_NO_AFFINE_EXIT_FOLD=1 still folded ($n_kill report(s))"

# ---------------------------------------------------------------- contract 3
note "contract 3: folded nested loop, folded type-top bound, pinned refusals"
body() { awk -v sym="^$2:" ' $0 ~ sym { inside=1 } inside { print } inside && /\.size/ { exit }' "$1"; }
"$CCC" -O2 -march=x86-64-v3 -S "$corpus" -o "$work/fold.s" || bad "-S build failed"

nested=$(body "$work/fold.s" nested_two)
grep -qE "cmpq \\\$4092, " <<< "$nested" \
    || bad "nested_two: no folded compare (\`cmpq \$4092\`) — the nested inner loop is not folding"
grep -qE "leaq 4\(" <<< "$nested" \
    && bad "nested_two: the offset materialisation (\`leaq 4(...)\`) survived"

high=$(body "$work/fold.s" near_high)
grep -q "9223372036854775803" <<< "$high" \
    || bad "near_high: no folded type-top bound (LLONG_MAX - 4) in the object code"
grep -qE "leaq 4\(" <<< "$high" \
    && bad "near_high: the offset materialisation (\`leaq 4(...)\`) survived"

# Self-loop and guard-only shapes: the phi's incomings are classified by value
# (the increment is the incoming that is `phi + Const`), so both fold even
# though the "entry" predecessor is the loop header itself.
sl=$(body "$work/fold.s" selfloop)
grep -qE "leaq 4\(" <<< "$sl" \
    && bad "selfloop: the offset materialisation (\`leaq 4(...)\`) survived"
grep -q "4096" <<< "$sl" \
    && bad "selfloop: the unfolded bound (\$4096) is still compared"
wg=$(body "$work/fold.s" while_guard)
grep -qE "leaq 4\(" <<< "$wg" \
    && bad "while_guard: the offset materialisation (\`leaq 4(...)\`) survived"
grep -q "2048" <<< "$wg" \
    && bad "while_guard: the unfolded bound (\$2048) is still compared"

rtb=$(body "$work/fold.s" refuse_rt_bound)
grep -qE "leaq 4\(" <<< "$rtb" \
    || bad "refuse_rt_bound: expected the unfolded offset (runtime bound must not fold)"
rts=$(body "$work/fold.s" refuse_rt_start)
grep -qE "leaq 4\(" <<< "$rts" \
    || bad "refuse_rt_start: expected the unfolded offset (runtime seed must not fold)"

# ── contract 4: the wraparound probes ────────────────────────────────────────
# The fold's licence is signed-no-overflow: `iv + C` must not wrap for the
# equivalence to hold.  These two probes are the shapes where a signed ADD
# would be UB but the SOURCE is an unsigned add feeding a signed cast -- the IR
# cannot tell the two apart (no nsw flag), so they are exactly the inputs on
# which a careless fold goes wrong.  Each probe carries its own reference
# implementation and prints MISMATCH/MISCOMPILE if the fold moved a value, so
# the contract is a stdout comparison rather than an assembly shape: the pass
# may fold or refuse per case, and either is fine as long as the ANSWER holds.
for probe in wrap_affine_fold wrap_loop_fold; do
    src="tests/regression/$probe.c"
    "$CCC" -O2 -o "$work/$probe.bin" "$src" || bad "$probe: compile failed"
    out=$("$work/$probe.bin") || bad "$probe: exit status $?"
    grep -q "MISCOMPILE\|MISMATCH" <<< "$out" && bad "$probe: $out"
    grep -q "exact" <<< "$out" || bad "$probe: unexpected output: $out"
    # Same answer with the fold killed: the probe must not depend on the fold.
    CCC_NO_AFFINE_EXIT_FOLD=1 "$CCC" -O2 -o "$work/$probe.off.bin" "$src" \
        || bad "$probe: compile failed under the kill switch"
    out_off=$("$work/$probe.off.bin") || bad "$probe (kill switch): exit status $?"
    [[ "$out" == "$out_off" ]] \
        || bad "$probe: kill switch changed the answer ($out vs $out_off)"
    echo "  wrap probe $probe: exact, and identical with the fold killed"
done

python3 "$here/affine_control_model.py"
# -fwrapv gives both signed-overflow witnesses defined wrapping semantics.
# Known outputs are pinned as well as GCC agreement; the disabled arm isolates
# the affine pass rather than accepting a shared wrong optimized answer.
for mode in -m64 -m32; do
    for opt in -O0 -O1 -O2 -O3 -Os; do
        src=tests/regression/affine_control_proof.c
        "${GCC:-gcc}" "$mode" "$opt" -fwrapv -Wall -Wextra -Werror "$src" -o "$work/control.ref"
        "$work/control.ref" > "$work/control.expected"
        grep -qx 'affine-control-proof 5 97' "$work/control.expected" || bad "control-proof: bad oracle"
        for rotated in 0 1; do
            for disabled in 0 1; do
                # Unset both for a clean default; AFFOLD is presence-based, so "0" still disables it.
                control_env=(env -u CCC_NO_AFFINE_EXIT_FOLD -u CCC_LOOP_ROTATE)
                if [[ $rotated == 1 ]]; then control_env+=(CCC_LOOP_ROTATE=1); fi
                if [[ $disabled == 1 ]]; then control_env+=(CCC_NO_AFFINE_EXIT_FOLD=1); fi
                CCC_VERIFY_IR=abort CCC_VALIDATE_SSA=1 "${control_env[@]}" \
                    "$CCC" "$mode" "$opt" -fwrapv "$src" -o "$work/control.mine"
                timeout 10 "$work/control.mine" > "$work/control.actual" || bad "control-proof $mode $opt rotated=$rotated disabled=$disabled: runtime failed"
                cmp "$work/control.expected" "$work/control.actual" || bad "control-proof $mode $opt rotated=$rotated disabled=$disabled: mismatch"
            done
        done
    done
done
# Retain useful clone folding when the original whole-loop proof licenses it.
# Fold reports use the [AFFOLD] grammar under CCC_DEBUG_AFFINE_FOLD; the early
# unsigned canonicalizer's "(early unsigned)" reports are a different
# mechanism and must not satisfy this rotation-clone assertion.
CCC_LOOP_ROTATE=1 CCC_DEBUG_LOOP_ROTATE=1 CCC_DEBUG_AFFINE_FOLD=1 \
    "$CCC" -O2 -fwrapv -S "$corpus" -o "$work/proved-rotation.s" 2> "$work/proved-rotation.log"
grep -E '\[AFFOLD\] [A-Za-z_][A-Za-z0-9_]*: affine exit-compare folds: [1-9]' "$work/proved-rotation.log" \
    | grep -vF '(early unsigned)' | grep -q . || bad "no proven rotation-clone fold survived"
echo '  signed control proof: m64/m32 oracle/rotation/disable matrix complete (verdict below)'

if [[ $fail -ne 0 ]]; then
    echo "check_affine_loop_fold: FAILED" >&2
    exit 1
fi
echo "check_affine_loop_fold: PASS (oracle parity, default-on counted fold, nested+type-top folds, refusals pinned)"
