#!/usr/bin/env bash
# ============================================================================
# check_affine_exit_compare.sh — ZERO-ROT-AFFINE: the rotated latch compares a
# bare IV against a FOLDED bound, and the compare-branch fusion accepts it.
#
# Why this gate exists.  `loop_rotate` clones the header guard into the latch,
# so the cloned test is `add(iv, C) < N` and the backend materialises `+C` as a
# fresh temporary on every iteration (the blocker measured in
# engineering/FOLLOWUP-2026-09-30-affine-exit-compare.md: 5 insns/iter against
# GCC's 4).  `canonicalise_affine_exit_cmps` folds the constant into the bound
# (`iv < N - C`, signed compares only), which both removes the temporary and
# gives the compare-branch fusion a bare-register compare to fuse — the second
# half only works because FileLiveness's backward-edge analysis became
# region-relative (the stale full-analysis answer made the fusion refuse the
# rotated setcc shape outright).
#
# The pass is opt-in (`CCC_LOOP_ROTATE=1`), so the gate drives it explicitly
# and pins three contracts:
#
#   1. CORRECTNESS. `affine_exit_compare.c` (constant-bound, runtime-bound,
#      pointer-IV, unsigned-refusal, define-mimic and already-canonical shapes,
#      references read through `volatile`) must agree with the GCC oracle on
#      stdout AND exit status with rotation on, with rotation off, and under
#      the kill switch.
#
#   2b. THE KILL SWITCH REACHES THE CLONE.  With rotation ON and
#      `CCC_NO_AFFINE_EXIT_FOLD=1`, the rotation path must report ZERO clone
#      folds while still reporting rotation candidates (otherwise the assertion
#      would hold for the wrong reason).
#
#   2. THE ROTATION CLONE IS STILL COUNTED.  Under `CCC_DEBUG_LOOP_ROTATE=1`
#      the rotation path must report at least one `[ROT] affine exit-compare
#      folds:` and none when rotation is off.  The fold is now ALSO a
#      default-on phase of its own (`[AFFOLD]` reports, gated by
#      tests/regression/check_affine_loop_fold.sh), so this contract is matched
#      on the rotation prefix alone: the two must not be confused.
#
#   3. THE OBJECT CODE IS THE 4-INSTRUCTION LOOP.  In `f_const` (`i + 4 <
#      2048`) the rotated build must compare the IV against the folded bound
#      directly and branch on it (`cmpq $2044, %rdx` followed by the
#      conditional jump — no setcc relay, no `test`), with no `leaq 4(` offset
#      materialisation and no `$2048` anywhere; the unrotated build must show
#      the 5-instruction shape it replaces (`leaq 4(%rdx)` + `cmpq $2048`).
#      That difference IS the win: the gate fails if the fold silently stops
#      applying, not just if the answer changes.
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
GCC=${GCC:-gcc}
[[ -x $CCC ]] || { echo "check_affine_exit_compare: lccc not found at $CCC" >&2; exit 1; }
command -v "$GCC" >/dev/null 2>&1 || { echo "check_affine_exit_compare: no gcc oracle" >&2; exit 1; }

corpus=$here/affine_exit_compare.c
work=$(mktemp -d "${TMPDIR:-/tmp}/lccc-affine.XXXXXX")
trap 'rm -rf "$work"' EXIT
fail=0
note() { printf '  %s\n' "$*"; }
bad() { printf '  FAIL: %s\n' "$*" >&2; fail=1; }

# ---------------------------------------------------------------- contract 1
note "contract 1: correctness against the GCC oracle (rotation on/off/killed)"
"$GCC" -O2 -march=x86-64-v3 "$corpus" -o "$work/ref" || bad "gcc oracle failed to build"
expected=$(timeout 300 "$work/ref") || bad "gcc oracle exited non-zero"
case $expected in
    *"affine-exit-compare: all shapes exact"*) ;;
    *) bad "gcc oracle did not report the expected success line" ;;
esac

run_case() { # name  "compile flags"  "RUN env assignments"
    local name=$1 flags=$2 envs=$3
    local -a farr=() earr=()
    read -r -a farr <<< "$flags"
    read -r -a earr <<< "$envs"
    if ! "$CCC" "${farr[@]}" "$corpus" -o "$work/$name" > "$work/$name.build" 2>&1; then
        bad "$name: lccc failed to build"; return
    fi
    local out rc=0
    if [[ ${#earr[@]} -gt 0 ]]; then
        out=$(timeout 300 env "${earr[@]}" "$work/$name") || rc=$?
    else
        out=$(timeout 300 "$work/$name") || rc=$?
    fi
    [[ $rc -eq 0 ]] || { bad "$name: exited $rc (expected 0)"; return; }
    [[ $out == "$expected" ]] || bad "$name: stdout differs from the gcc oracle"
}

run_case "rot-O2" "-O2 -march=x86-64-v3" "CCC_LOOP_ROTATE=1"
run_case "rot-O3" "-O3 -march=x86-64-v3" "CCC_LOOP_ROTATE=1"
run_case "rot-O1" "-O1 -march=x86-64-v3" "CCC_LOOP_ROTATE=1"
run_case "norot-O2" "-O2 -march=x86-64-v3" ""
run_case "killed-O2" "-O2 -march=x86-64-v3" "CCC_LOOP_ROTATE=1 CCC_NO_LOOP_ROTATE=1"

# ---------------------------------------------------------------- contract 2
note "contract 2: the rotation clone's affine fold is counted"
CCC_LOOP_ROTATE=1 CCC_DEBUG_LOOP_ROTATE=1 "$CCC" -O2 -march=x86-64-v3 -S "$corpus" \
    -o "$work/rot.s" 2> "$work/rot.dbg" || bad "rotated -S build failed"
CCC_DEBUG_LOOP_ROTATE=1 "$CCC" -O2 -march=x86-64-v3 -S "$corpus" \
    -o "$work/norot.s" 2> "$work/norot.dbg" || bad "unrotated -S build failed"
folds=$(grep -cF "[ROT] affine exit-compare folds:" "$work/rot.dbg" || true)
folds_off=$(grep -cF "[ROT] affine exit-compare folds:" "$work/norot.dbg" || true)
note "rotated: $folds rotation-clone fold report(s); unrotated: $folds_off"
[[ $folds -gt 0 ]] || bad "no affine exit-compare fold fired inside rotation"
[[ $folds_off -eq 0 ]] || bad "rotation reported a clone fold without running ($folds_off report(s))"

# The kill switch must reach BOTH producers of the fold.  `CCC_NO_AFFINE_EXIT_FOLD`
# exists so a misbehaving fold can be disabled without disabling rotation (an
# older, separate transformation), and it used to gate only the standalone pass:
# the rotation clone kept folding, which made the A/B that condemns the fold the
# same A/B that hides it.  The count is the observable, so assert on it.
CCC_LOOP_ROTATE=1 CCC_NO_AFFINE_EXIT_FOLD=1 CCC_DEBUG_LOOP_ROTATE=1 "$CCC" \
    -O2 -march=x86-64-v3 -S "$corpus" -o "$work/rotkilled.s" 2> "$work/rotkilled.dbg" \
    || bad "rotated kill-switch -S build failed"
folds_killed=$(grep -cF "[ROT] affine exit-compare folds:" "$work/rotkilled.dbg" || true)
note "rotated with CCC_NO_AFFINE_EXIT_FOLD=1: $folds_killed rotation-clone fold report(s)"
[[ $folds_killed -eq 0 ]] \
    || bad "CCC_NO_AFFINE_EXIT_FOLD left the rotation clone fold running ($folds_killed report(s))"
# ... and rotation itself still ran, or the line above would be vacuous.
grep -qF "[ROT] candidate:" "$work/rotkilled.dbg" \
    || bad "rotation did not run under the fold kill switch (test would be vacuous)"

# ---------------------------------------------------------------- contract 3
note "contract 3: f_const is the folded loop rotated and unrotated"
body() { awk -v sym="^$2:" ' $0 ~ sym { inside=1 } inside { print } inside && /\.size/ { exit }' "$1"; }
rot_body=$(body "$work/rot.s" f_const)
norot_body=$(body "$work/norot.s" f_const)

grep -qE "cmpq \\\$2044, %rdx" <<< "$rot_body" \
    || bad "rotated f_const: no folded compare (\`cmpq \$2044, %rdx\`)"
grep -qE "leaq 4\(" <<< "$rot_body" \
    && bad "rotated f_const: the offset materialisation (\`leaq 4(...)\`) is still there"
grep -q "2048" <<< "$rot_body" \
    && bad "rotated f_const: the unfolded bound (\$2048) is still compared"
grep -qE "^[[:space:]]+(set[a-z]+|test)[bwlq]?[[:space:]]" <<< "$rot_body" \
    && bad "rotated f_const: a setcc/test relay survived — the compare is not fused"
# The folded compare must branch directly (fused jcc on the bare IV).
# The folded compare must branch directly (fused jcc on the bare IV).  One
# awk over the captured body, not a `grep | grep -q` pipeline: the
# pipefail-sigpipe gate rejects pipelines whose consumer may exit early.
awk '/cmpq \$2044, %rdx/ { getline nxt; if (nxt ~ /^[[:space:]]*j/) ok = 1 }\
     END { exit ok ? 0 : 1 }' <<< "$rot_body" \
    || bad "rotated f_const: the folded compare does not feed a conditional jump"

# The standalone pass folds the unrotated loop as well: both compares (the
# entry guard and the loop-back test) must carry the folded bound, and the
# offset materialisation must be gone from the function.
grep -qE "leaq 4\(" <<< "$norot_body" \
    && bad "unrotated f_const: the offset materialisation (\`leaq 4(...)\`) survived — the default-on fold is not firing"
grep -q "2048" <<< "$norot_body" \
    && bad "unrotated f_const: the unfolded bound (\$2048) is still compared"
grep -qE "cmpq \\\$2044, %rdx" <<< "$norot_body" \
    || bad "unrotated f_const: no folded compare (\`cmpq \$2044, %rdx\`)"

if [[ $fail -ne 0 ]]; then
    echo "check_affine_exit_compare: FAILED" >&2
    exit 1
fi
echo "check_affine_exit_compare: PASS (oracle parity, counted rotation-clone fold, f_const folded rotated and unrotated)"
