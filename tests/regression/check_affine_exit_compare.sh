#!/usr/bin/env bash
# Affine bounds after rotation must use the same sequence proof as ordinary
# loops. There is no separate unchecked rotation-clone shortcut: it miscompiled
# defined -fwrapv loops. Check actual compiler environments, reference output,
# counted proof-based folds, the kill switch, and the original hot-loop shape.
# Rotation must still run when affine folding is disabled, otherwise the kill
# control would be vacuous. Both rotated and unrotated constant-bound loops
# should use a bare-register folded compare feeding the branch directly.
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

run_case() { # name  "compile flags"  "compiler env assignments"
    local name=$1 flags=$2 envs=$3
    local -a farr=() earr=()
    read -r -a farr <<< "$flags"
    read -r -a earr <<< "$envs"
    if ! env "${earr[@]}" "$CCC" "${farr[@]}" "$corpus" -o "$work/$name" > "$work/$name.build" 2>&1; then
        bad "$name: lccc failed to build"; return
    fi
    local out rc=0
    out=$(timeout 300 "$work/$name") || rc=$?
    [[ $rc -eq 0 ]] || { bad "$name: exited $rc (expected 0)"; return; }
    [[ $out == "$expected" ]] || bad "$name: stdout differs from the gcc oracle"
}

run_case "rot-O2" "-O2 -march=x86-64-v3" "CCC_LOOP_ROTATE=1"
run_case "rot-O3" "-O3 -march=x86-64-v3" "CCC_LOOP_ROTATE=1"
run_case "rot-O1" "-O1 -march=x86-64-v3" "CCC_LOOP_ROTATE=1"
run_case "norot-O2" "-O2 -march=x86-64-v3" ""
run_case "killed-O2" "-O2 -march=x86-64-v3" "CCC_LOOP_ROTATE=1 CCC_NO_LOOP_ROTATE=1"

# ---------------------------------------------------------------- contract 2
note "contract 2: the shared proof-based affine fold is counted"
CCC_LOOP_ROTATE=1 CCC_DEBUG_LOOP_ROTATE=1 CCC_DEBUG_AFFINE_FOLD=1 "$CCC" -O2 -march=x86-64-v3 -S "$corpus" \
    -o "$work/rot.s" 2> "$work/rot.dbg" || bad "rotated -S build failed"
CCC_DEBUG_LOOP_ROTATE=1 CCC_DEBUG_AFFINE_FOLD=1 "$CCC" -O2 -march=x86-64-v3 -S "$corpus" \
    -o "$work/norot.s" 2> "$work/norot.dbg" || bad "unrotated -S build failed"
folds=$(grep -cF "[AFFOLD]" "$work/rot.dbg" || true)
folds_off=$(grep -cF "[AFFOLD]" "$work/norot.dbg" || true)
note "rotated: $folds proof-based fold report(s); unrotated: $folds_off"
[[ $folds -gt 0 ]] || bad "no proof-based affine exit-compare fold fired after rotation"
[[ $folds_off -gt 0 ]] || bad "the proof-based fold did not run without rotation"

# One owner, one kill switch. Rotation must remain active under this control.
CCC_LOOP_ROTATE=1 CCC_NO_AFFINE_EXIT_FOLD=1 CCC_DEBUG_LOOP_ROTATE=1 CCC_DEBUG_AFFINE_FOLD=1 "$CCC" \
    -O2 -march=x86-64-v3 -S "$corpus" -o "$work/rotkilled.s" 2> "$work/rotkilled.dbg" \
    || bad "rotated kill-switch -S build failed"
folds_killed=$(grep -cF "[AFFOLD]" "$work/rotkilled.dbg" || true)
note "rotated with CCC_NO_AFFINE_EXIT_FOLD=1: $folds_killed proof-based fold report(s)"
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
echo "check_affine_exit_compare: PASS (oracle parity, counted proof-based fold, f_const folded rotated and unrotated)"
