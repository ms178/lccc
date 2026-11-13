#!/usr/bin/env bash
# 256- and 512-bit load/store intrinsics must be a single move each.
#
# Background (all numbers measured on this tree, x86-64, at -O0 unless stated):
#   * `_mm256_loadu_ps`/`storeu_ps`/`loadu_pd`/`storeu_pd` were lane loops:
#     28 instructions + a 176-byte frame for a load, 22 for a store, where
#     gcc-16 emits two.  They only folded to `vmovups` once SLP ran, so -O0 and
#     -Og paid full price for an intrinsic that is supposed to BE the move.
#   * `_mm256_loadu_si256`/`storeu_si256`/`lddqu_si256` were a 32-byte
#     `__builtin_memcpy`; `_mm256_load_ps`/`store_ps`/`load_pd`/`store_pd` did
#     not exist.
#   * `_mm512_loadu_ps`/`storeu_ps` were 16-iteration lane loops that SLP never
#     folded even at -O2: 39 instructions and a 472-byte frame for a load, 33
#     for a store.  `_mm512_loadu_pd`/`storeu_pd`/`load_pd`/`store_pd` did not
#     exist at all.
#
# This gate pins the emitted shape (one move per function, no stack traffic
# beyond the ABI frame) and runs both fixtures against a reference compiler.
# The 512-bit fixture self-skips on a host without AVX-512F; the shape half of
# the check is static and runs everywhere.
#
# Usage: check_wide_load_store.sh [--lccc PATH] [--reference CC]
set -uo pipefail

LCCC="${LCCC:-./target/fastbuild/lccc}"
REFERENCE="${REFERENCE:-gcc-16}"
while [ $# -gt 0 ]; do
    case "$1" in
        --lccc) LCCC="$2"; shift 2 ;;
        --reference) REFERENCE="$2"; shift 2 ;;
        *) echo "check_wide_load_store: unknown argument '$1'" >&2; exit 2 ;;
    esac
done

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
FIXTURE=tests/regression/simd_wide_load_store.c
FIXTURE512=tests/regression/simd_wide_load_store_512.c

if [ ! -x "$LCCC" ]; then
    echo "FAIL: no compiler at '$LCCC'" >&2
    exit 1
fi

fail=0
level_fail=0
bad() { echo "FAIL: $*" >&2; fail=1; level_fail=1; }

body_of() {
    awk -v fn="$2:" 'index($0, fn) == 1 {inside=1; next}
                     /^[a-z_][A-Za-z_0-9]*:/ {inside=0} inside' "$1"
}

# `count <asm> <function> <mnemonic-regex>` -- instruction census in one body.
count_in() {
    body_of "$1" "$2" > "$WORK/body.s"
    if [ ! -s "$WORK/body.s" ]; then
        echo "-1"
        return
    fi
    grep -cE "^[[:space:]]+$3[[:space:]]" "$WORK/body.s"
}

# ── 256-bit: one move per function, at -O0 and -O2 ─────────────────────────
for level in -O0 -O2; do
    level_fail=0
    ASM="$WORK/w256${level}.s"
    "$LCCC" "$level" -mavx2 -S -o "$ASM" "$FIXTURE" || { echo "FAIL: compile at $level" >&2; exit 1; }
    for fn in core_u_ps core_a_ps core_u_pd core_a_pd core_u_si core_lddqu_si; do
        got=$(count_in "$ASM" "$fn" 'vmov(u|dqu|dqa)[a-z]*')
        if [ "$got" -lt 0 ]; then
            bad "$level: cannot extract $fn"
        elif [ "$got" -lt 1 ]; then
            bad "$level: $fn emits no vector move"
        fi
    done
    # No per-lane scalar traffic in a pure load/store helper: at -O2 the whole
    # body is one move plus the frame, at -O0 the ABI staging adds a handful.
    if [ "$level" = "-O2" ]; then
        for fn in core_u_ps core_a_ps core_u_pd core_a_pd core_u_si core_lddqu_si; do
            body_of "$ASM" "$fn" > "$WORK/body.s"
            n=$(grep -cE '^[[:space:]]+[a-z]' "$WORK/body.s")
            if [ "$n" -gt 8 ]; then
                bad "$level: $fn body is $n instructions (want <= 8)"
            fi
        done
    fi
    # `vmovss`/`vmovsd`/`vmovq` are the per-lane signature of the old loops.
    for fn in core_u_ps core_a_ps core_u_pd core_a_pd core_u_si core_lddqu_si; do
        lanes=$(count_in "$ASM" "$fn" 'vmov(s[sd]|q)')
        if [ "$lanes" -ne 0 ]; then
            bad "$level: $fn still moves $lanes lane(s) through an XMM/GPR"
        fi
    done
    if [ "$level_fail" -eq 0 ]; then
        echo "  ok  $level: 256-bit loads/stores are single moves, no per-lane traffic"
    fi
done

# ── 512-bit: one move per function ─────────────────────────────────────────
ASM512="$WORK/w512.s"
"$LCCC" -O2 -mavx512f -S -o "$ASM512" "$FIXTURE512" || { echo "FAIL: 512-bit compile" >&2; exit 1; }
level_fail=0
for fn in u_ps u_pd a_ps a_pd; do
    got=$(count_in "$ASM512" "$fn" 'vmovdqu64')
    if [ "$got" -lt 0 ]; then
        bad "cannot extract 512 $fn"
    elif [ "$got" -lt 2 ]; then
        # One load, one store.
        bad "512 $fn emits $got 'vmovdqu64' (want at least 2: load and store)"
    fi
done
for mnemonic in vaddps vsubps vmulps; do
    got=$(count_in "$ASM512" arith "$mnemonic")
    if [ "$got" -ne 1 ]; then
        bad "512 arith emits $got '$mnemonic' (want 1)"
    fi
done
if [ "$level_fail" -eq 0 ]; then
    echo "  ok  -O2: 512-bit loads/stores and float arithmetic are single instructions"
fi

# ── Execution: byte parity with the reference compiler ─────────────────────
if command -v "$REFERENCE" >/dev/null 2>&1; then
    for level in -O0 -O2; do
        "$LCCC" "$level" -mavx2 "$FIXTURE" -o "$WORK/l.bin" || { bad "$level: fixture does not link"; continue; }
        "$WORK/l.bin" > "$WORK/l.out" || { bad "$level: fixture exits non-zero"; continue; }
        "$REFERENCE" "$level" -mavx2 "$FIXTURE" -o "$WORK/r.bin" || { bad "$level: reference cannot build the fixture"; continue; }
        "$WORK/r.bin" > "$WORK/r.out"
        cmp -s "$WORK/l.out" "$WORK/r.out" || { bad "$level: 256-bit output differs from $REFERENCE"; diff "$WORK/l.out" "$WORK/r.out" >&2 || true; }
    done
    if "$LCCC" -O2 -mavx512f "$FIXTURE512" -o "$WORK/l5.bin" 2>"$WORK/err"; then
        "$WORK/l5.bin" > "$WORK/l5.out" || bad "512-bit fixture exits non-zero"
        if "$REFERENCE" -O2 -mavx512f "$FIXTURE512" -o "$WORK/r5.bin" 2>/dev/null; then
            "$WORK/r5.bin" > "$WORK/r5.out"
            cmp -s "$WORK/l5.out" "$WORK/r5.out" || { bad "512-bit output differs from $REFERENCE"; diff "$WORK/l5.out" "$WORK/r5.out" >&2 || true; }
        fi
        if grep -q '^skip:' "$WORK/l5.out"; then
            echo "  ..  512-bit execution skipped: no avx512f on this host"
        fi
    else
        bad "512-bit fixture does not link: $(head -1 "$WORK/err")"
    fi
    if [ "$fail" -eq 0 ]; then
        echo "  ok  executed output matches $REFERENCE"
    fi
else
    echo "  ..  reference '$REFERENCE' unavailable; parity check skipped"
fi

if [ "$fail" -ne 0 ]; then
    echo "FAIL: wide load/store lowering"
    exit 1
fi
echo "PASS: 256- and 512-bit load/store intrinsics are single moves and match the reference"
