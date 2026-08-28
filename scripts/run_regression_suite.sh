#!/usr/bin/env bash
# ============================================================================
# run_regression_suite.sh — LCCC regression suite runner with A/B differential
#
# For every tests/regression/*.c (optional .flags / .env sidecars):
#   1. Compile + run with the current lccc build.
#   2. Unless the .env sets LCCC_NO_COMPARE=1, also compile + run with GCC
#      and require identical stdout+exit code.
#   3. A/B DIFFERENTIAL: unless the .env sets LCCC_NO_AB=1, re-run every test
#      with CCC_NO_SMALL_SLOTS=1 and require identical output to the default
#      run.  Width-partitioned 4-byte spill slots must be value-preserving;
#      any divergence between the two configurations is a miscompile and
#      fails the suite.  (LCCC_NO_AB is for tests whose very subject is the
#      frame-size improvement itself, e.g. small_slot_frame_bloat: the
#      8-byte-slot configuration legitimately cannot survive the recursion
#      depth the fixed one survives.)
#
# Usage:
#   ./run_regression_suite.sh [filter-substring]
# Environment:
#   LCCC_BIN   compiler under test (default target/fastbuild/lccc)
#   GCC_BIN    oracle compiler    (default gcc)
# ============================================================================
set -u

REPO=${LCCC_REPO:-$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)}
REG="$REPO/tests/regression"
LCCC_BIN=${LCCC_BIN:-$REPO/target/fastbuild/lccc}
GCC_BIN=${GCC_BIN:-gcc}
GCC_INC="-I$(gcc -print-file-name=include)"
FILTER=${1:-}
WORK=$(mktemp -d /tmp/lccc-reg.XXXXXX)
trap 'rm -rf "$WORK"' EXIT

pass=0; fail=0; skip=0; ab_fail=0
declare -a FAILED=()

run_one() {  # run_one <src> ; env may override CCC_NO_SMALL_SLOTS etc.
    local src=$1
    local base=${src%.c}
    local flags=()
    [[ -f "$base.flags" ]] && read -r -a flags < "$base.flags"
    local obj="$WORK/$(basename "$base").bin"
    if ! "$LCCC_BIN" $GCC_INC -O2 "${flags[@]}" "$src" -o "$obj" 2>"$WORK/cc.err"; then
        echo "BUILDFAIL"; return
    fi
    local out
    out=$(timeout 20 "$obj" 2>&1); local ec=$?
    [[ $ec -eq 124 ]] && { echo "TIMEOUT"; return; }
    echo "$out|$ec"
}

for src in "$REG"/*.c; do
    name=$(basename "$src" .c)
    [[ -n $FILTER && $name != *$FILTER* ]] && continue

    # per-test env (LCCC_NO_COMPARE=1: lccc-only semantics; LCCC_NO_AB=1:
    # exempt from the small-slot A/B differential, see header)
    env_vars=()
    if [[ -f "$REG/$name.env" ]]; then
        while IFS= read -r line; do
            [[ -z $line || $line == \#* ]] && continue
            env_vars+=("$line")
        done < "$REG/$name.env"
    fi
    no_compare=0; no_ab=0
    for e in "${env_vars[@]:-}"; do
        [[ $e == "LCCC_NO_COMPARE=1" ]] && no_compare=1
        [[ $e == "LCCC_NO_AB=1" ]] && no_ab=1
    done

    # 1. lccc run (default configuration)
    res=$(env ${env_vars[@]:-} bash -c "$(declare -f run_one); LCCC_BIN='$LCCC_BIN' GCC_INC='$GCC_INC' WORK='$WORK'; run_one '$src'")

    if [[ $res == "BUILDFAIL" ]]; then
        echo "FAIL  $name (lccc build failed: $(head -3 "$WORK/cc.err" | tr '\n' ' '))"
        fail=$((fail+1)); FAILED+=("$name:build"); continue
    fi
    if [[ $res == "TIMEOUT" ]]; then
        echo "FAIL  $name (timeout)"
        fail=$((fail+1)); FAILED+=("$name:timeout"); continue
    fi

    # 2. GCC oracle comparison
    if [[ $no_compare -eq 0 ]]; then
        gflags=()
        [[ -f "$REG/$name.flags" ]] && read -r -a gflags < "$REG/$name.flags"
        gbin="$WORK/$(basename "$name").gcc.bin"
        if ! "$GCC_BIN" -O2 "${gflags[@]}" "$src" -o "$gbin" -lm 2>/dev/null; then
            skip=$((skip+1))   # GCC can't build it (lccc-specific asm) — lccc-only test
        else
            gout=$(timeout 20 "$gbin" 2>&1); gec=$?
            if [[ "$gout|$gec" != "$res" ]]; then
                echo "FAIL  $name (GCC mismatch)"
                echo "      gcc : $(echo "$gout|$gec" | head -2)"
                echo "      lccc: $(echo "$res" | head -2)"
                fail=$((fail+1)); FAILED+=("$name:oracle"); continue
            fi
        fi
    fi

    # 3. A/B differential: small slots on (default) vs off
    if [[ $no_ab -eq 0 ]]; then
        res_ab=$(env ${env_vars[@]:-} CCC_NO_SMALL_SLOTS=1 bash -c "$(declare -f run_one); LCCC_BIN='$LCCC_BIN' GCC_INC='$GCC_INC' WORK='$WORK'; run_one '$src'")
        if [[ $res_ab != "$res" ]]; then
            echo "FAIL  $name (A/B small-slot differential)"
            echo "      default : $(echo "$res" | head -2)"
            echo "      nosmall : $(echo "$res_ab" | head -2)"
            fail=$((fail+1)); ab_fail=$((ab_fail+1)); FAILED+=("$name:ab"); continue
        fi
    fi

    pass=$((pass+1))
done

echo
echo "================================================================"
echo "regression suite: PASS=$pass FAIL=$fail SKIP=$skip (AB-diff failures: $ab_fail)"
if [[ ${#FAILED[@]} -gt 0 ]]; then printf 'failed: %s\n' "${FAILED[@]}"; fi
echo "================================================================"

# ── 32 KiB boot-code size gate (the e12597c7 lesson: a size regression in
# the boot path went unnoticed for sessions because nothing measured it).
# Metric: pre-pecompat content end ≤ 24,576 (the alignment-cliff boundary;
# `.pecompat` forces 4096 alignment, so `_end` jumps in 4 KiB steps and
# hides progress inside a step). Skipped entirely when the prepared kernel
# tree is absent (bare CI containers) — a skip is reported, not a pass.
if [[ -n "${KERNEL_DIR:-}" && -f "$KERNEL_DIR/arch/x86/boot/printf.c" ]]; then
    boot_out="$WORK/bootgate"
    # build_kernel_boot.sh exits non-zero when the 32 KiB gate itself fails —
    # that is a MEASUREMENT, not a build error; parse the log either way.
    boot_log=$(KERNEL_DIR="$KERNEL_DIR" \
               LCCC="$REPO/target/fastbuild/lccc" \
               LCCC_LD="$REPO/target/fastbuild/lccc-ld" \
               OUT="$boot_out" \
               timeout 600 bash "$REPO/scripts/build_kernel_boot.sh" 2>&1 || true)
    text_size=$(echo "$boot_log" | awk '$1 == ".text" && $2 ~ /^[0-9]+$/ {s=$2} END {print s}')
    if [[ -z "$text_size" ]]; then
        echo "boot gate: FAIL (no .text measurement — build errored)"
        echo "$boot_log" | tail -5
        fail=$((fail+1))
    else
        # content_end = 1,166 (bstext..initdata) + .text + 30 (.text32)
        content_end=$(( 1166 + text_size + 30 ))
        if (( content_end <= 24576 )); then
            echo "boot gate: PASS (pre-pecompat end $content_end ≤ 24,576; .text $text_size)"
        else
            echo "boot gate: FAIL (pre-pecompat end $content_end > 24,576; .text $text_size)"
            echo "          .text budget: ≤ 23,380 — see updates/followup_2026-08-27_session06.md §3"
            fail=$((fail+1))
        fi
    fi
else
    echo "boot gate: SKIP (KERNEL_DIR not set or tree not prepared)"
fi

[[ $fail -eq 0 ]]
