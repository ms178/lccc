#!/usr/bin/env bash
# ============================================================================
# check_vec_chain_exit_phi.sh — ZERO-REM-2 + chained-exit-phi FMA contract:
# the hoisted FMA transform may only be emitted where the A factor is
# re-established in its FIXED register carrier on EVERY entry edge.
#
# Why this gate exists.  The k-unrolled matmul shapes CHAIN their j-loops: the
# exit block of j-loop N is the HEADER of j-loop N+1, and the next header is
# entered on the false arm of the previous loop's header.  The hoist used to
# look for a block whose terminator was an unconditional `Branch` into the
# header, found none for loops 2..K, and then emitted the hoisted FMAs anyway.
# `BroadcastLoadF64` and `FmaF64x4HoistedSIB` are glued through %ymm1 (a fixed
# register with no SSA edge between them), so the later k-loops multiplied by
# the first loop's A[i][0] -- a wrong-A miscompile, measured as C[0][0] = 1006
# instead of 1003 pre-fix, 1030 (no A contribution at all) before the
# remainder/exit-phi work.  Fix: hoist onto every entry edge, splitting a
# conditional entry edge into its own block; refuse the transform when no
# entry edge can be isolated.
#
# Three contracts, pinned separately because each can fail alone:
#
#   1. CORRECTNESS. `matmul_chain_exit_phi.c` must agree with the GCC oracle
#      on stdout AND exit status at -O0..-O3 on AVX2+FMA (-march=x86-64-v3),
#      under the two-wide lowering (LCCC_FORCE_SSE2=1), and against the same
#      compiler with the vectorizer off (CCC_NO_MAP_VEC=1) and with the
#      ZERO-REM-2 elision off (CCC_NO_MAP_ZERO_REM=1).  Every shape compares
#      whole rows including the 16-element padding sentinel, so a loop that
#      runs one chunk too far or stops one chunk early is visible.
#
#   2. THE CARRIER CONTRACT HOLDS STRUCTURALLY.  One packed loop = one
#      broadcast + 4 SIB chunks (quad), or one broadcast + 1 two-wide FMA.
#      The object code must show `fma == 4*bc` / `fma == bc` for every shape
#      that vectorizes at all.  Pre-fix: 16 FMA against 1 broadcast.
#
#   3. EVERY TRANSFORM IS EITHER BROADCAST OR REFUSED.  Under
#      LCCC_DEBUG_VECTORIZE=1 each "Matmul pattern matched" must be followed
#      by a "Hoisted BroadcastLoadF64" (or an explicit refusal) before the
#      next match, and the ZERO-REM-2 elision must fire on the exact-multiple
#      shapes while vanishing under its kill switch.
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
GCC=${GCC:-gcc}
[[ -x $CCC ]] || { echo "check_vec_chain_exit_phi: lccc not found at $CCC" >&2; exit 1; }
command -v "$GCC" >/dev/null 2>&1 || { echo "check_vec_chain_exit_phi: no gcc oracle" >&2; exit 1; }

corpus=$here/matmul_chain_exit_phi.c
arch=x86-64-v3
shapes=(mm_256_4 mm_48_5 mm_32_2 mm_16_4 mm_64_1 mm_255_4 mm_250_3 mm_17_4 mm_33_4 mm_300_2)
work=$(mktemp -d "${TMPDIR:-/tmp}/lccc-chainphi.XXXXXX")
trap 'rm -rf "$work"' EXIT
fail=0
note() { printf '  %s\n' "$*"; }
bad() { printf '  FAIL: %s\n' "$*" >&2; fail=1; }

# ---------------------------------------------------------------- contract 1
note "contract 1: correctness against the GCC oracle, both widths, kill switches"
"$GCC" -O2 "-march=$arch" "$corpus" -o "$work/ref" \
    || bad "gcc oracle failed to build"
expected=$(timeout 120 "$work/ref") || bad "gcc oracle exited non-zero"
case $expected in
    *"all shapes exact"*) ;;
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
        out=$(timeout 120 env "${earr[@]}" "$work/$name") || rc=$?
    else
        out=$(timeout 120 "$work/$name") || rc=$?
    fi
    if [[ $rc -ne 0 ]]; then
        bad "$name: exited $rc (expected 0)"; printf '%s\n' "$out" | sed -n '1,3p' >&2; return
    fi
    if [[ $out != "$expected" ]]; then
        bad "$name: stdout differs from the gcc oracle"
        printf '%s\n' "$out" | sed -n '1,3p' >&2
    fi
}

for opt in -O0 -O1 -O2 -O3; do
    run_case "quad${opt}" "$opt -march=$arch" ""
done
run_case "quad-Os" "-Os -march=$arch" ""
run_case "sse2-O2" "-O2 -march=$arch" "LCCC_FORCE_SSE2=1"
run_case "sse2-O3" "-O3 -march=$arch" "LCCC_FORCE_SSE2=1"
# Same compiler, vectorizer off: the scalar path is the cheapest differential
# oracle that isolates this transform.
run_case "no-vec-O2" "-O2 -march=$arch" "CCC_NO_MAP_VEC=1"
# Elision off (chained exit phis still relabelled): the shapes must agree with
# the elided build -- the omission may not change an answer.
run_case "no-zerorem-O2" "-O2 -march=$arch" "CCC_NO_MAP_ZERO_REM=1"

# ---------------------------------------------------------------- contract 2
note "contract 2: one broadcast per packed loop in the object code (fma == 4*bc / fma == bc)"
count_sym() { # binary symbol insn-regex
    objdump -d --no-show-raw-insn "$1" \
        | awk -v sym="<$2>:" -v pat="$3" '
            $0 ~ "^[0-9a-f]+ " sym { inside=1; next }
            inside && /^$/ { exit }
            inside && $0 ~ pat { n++ }
            END { print n+0 }'
}
"$CCC" -O2 "-march=$arch" "$corpus" -o "$work/obj-quad" || bad "quad build failed"
LCCC_FORCE_SSE2=1 "$CCC" -O2 "-march=$arch" "$corpus" -o "$work/obj-sse2" || bad "sse2 build failed"
vectorized=0
for s in "${shapes[@]}"; do
    for pair in "quad:$work/obj-quad:4" "sse2:$work/obj-sse2:1"; do
        IFS=: read -r label bin per <<< "$pair"
        bc=$(count_sym "$bin" "$s" 'vbroadcastsd')
        fma=$(count_sym "$bin" "$s" 'vfmadd231pd')
        if [[ $fma -eq 0 && $bc -ne 0 ]]; then
            bad "$label $s: broadcast with no packed FMA body ($bc/$fma)"
        elif [[ $fma -ne 0 && $((bc * per)) -ne $fma ]]; then
            bad "$label $s: $bc broadcast(s) for $fma packed FMA insns (expected fma == $per*bc)"
        fi
        if [[ $label == quad && $s != mm_16_4 && $fma -ne 0 ]]; then
            vectorized=$((vectorized + 1))
        fi
    done
done
note "quad shapes carrying the packed body: $vectorized (mm_16_4 is below the vectorizer's size floor)"
[[ $vectorized -ge 9 ]] || bad "only $vectorized/9 shapes vectorized -- the transform stopped firing"

# ---------------------------------------------------------------- contract 3
note "contract 3: every matched loop is broadcast or refused; elision obeys its kill switch"
LCCC_DEBUG_VECTORIZE=1 "$CCC" -O2 "-march=$arch" -S "$corpus" -o /dev/null 2> "$work/dbg" \
    || bad "debug build failed"
matched=$(grep -c "Matmul pattern matched" "$work/dbg" || true)
hoisted=$(grep -c "Hoisted BroadcastLoadF64" "$work/dbg" || true)
refused=$(grep -c "no isolatable entry edge" "$work/dbg" || true)
omitted=$(grep -c "Remainder omitted" "$work/dbg" || true)
note "matched=$matched hoisted=$hoisted refused=$refused remainder-omitted=$omitted"
[[ $matched -gt 0 ]] || bad "the matmul pattern never matched"
[[ $hoisted -ge $matched ]] || bad "matched loops without an A broadcast: %ymm1 would carry a stale factor"
[[ $refused -eq 0 ]] || bad "$refused transform(s) refused -- the corpus must stay vectorizable"
[[ $omitted -gt 0 ]] || bad "ZERO-REM-2 elision did not fire on the exact-multiple shapes"
awk '
    /Matmul pattern matched/ { if (pending) { print "  unmatched: " line; bad=1 } pending=1; line=$0; next }
    /Hoisted BroadcastLoadF64|no isolatable entry edge/ { pending=0 }
    END { if (pending) { print "  unmatched: " line; bad=1 } exit bad }
' "$work/dbg" || bad "a matched loop ended without a broadcast (or a refusal)"

CCC_NO_MAP_ZERO_REM=1 LCCC_DEBUG_VECTORIZE=1 "$CCC" -O2 "-march=$arch" -S "$corpus" -o /dev/null 2> "$work/dbg-nz" \
    || bad "kill-switch build failed"
omitted_nz=$(grep -c "Remainder omitted" "$work/dbg-nz" || true)
[[ $omitted_nz -eq 0 ]] || bad "CCC_NO_MAP_ZERO_REM=1 did not suppress the elision ($omitted_nz omissions)"
[[ $(grep -c "Retargeted" "$work/dbg" || true) -gt 0 ]] \
    || bad "the chained exit-phi retarget never fired -- the chain shapes did not reach it"

if [[ $fail -ne 0 ]]; then
    echo "check_vec_chain_exit_phi: FAILED" >&2
    exit 1
fi
echo "check_vec_chain_exit_phi: PASS (oracle parity, carrier ratio, broadcast-or-refuse, kill switches)"
