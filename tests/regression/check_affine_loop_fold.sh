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

run_case() { # name  "flags"  "RUN env assignments"
    local name=$1 flags=$2 envs=$3
    local -a farr=() earr=()
    read -r -a farr <<< "$flags"
    read -r -a earr <<< "$envs"
    if ! "$CCC" "${farr[@]}" "$corpus" -o "$work/$name" > "$work/$name.build" 2>&1; then
        bad "$name: lccc failed to build"; return
    fi
    local out rc=0
    if [[ ${#earr[@]} -gt 0 ]]; then
        out=$(timeout 900 env "${earr[@]}" "$work/$name") || rc=$?
    else
        out=$(timeout 900 "$work/$name") || rc=$?
    fi
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
count_reports() { # env...
    local out
    out=$(timeout 900 env "$@" "$CCC" -O2 -march=x86-64-v3 -S "$corpus" -o /dev/null 2>&1 >/dev/null || true)
    printf '%s\n' "$out" | grep -c "affine exit-compare folds:" || true
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

if [[ $fail -ne 0 ]]; then
    echo "check_affine_loop_fold: FAILED" >&2
    exit 1
fi
echo "check_affine_loop_fold: PASS (oracle parity, default-on counted fold, nested+type-top folds, refusals pinned)"
