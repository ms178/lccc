#!/usr/bin/env bash
# Gate: the i686 backend's 128-bit VEX FMA3 spellings compile AND compute a*b+c.
#
# The defect this exists for was a CRASH, not a wrong answer: every 128-bit
# spelling (`_mm_fmadd_ps`, `_mm_fmadd_pd` and the raw `__lccc_simd128_*`
# builtins) lowered to `IntrinsicOp::FmaPs132` + friends, the i686 dispatcher
# had no arm for any of them, and they fell through to the catch-all
# `reject_value_form`, which PANICS:
#
#     thread '<unnamed>' panicked at src/backend/i686/codegen/intrinsics.rs:
#     i686 emit_intrinsic: FmaPs132 produces a value (dest) but only the
#     dest_ptr form is supported here
#
# Measured reproducer: `lccc -m32 -mavx -mfma -c fma.c` on `_mm_fmadd_ps`.
# GCC 16.2 and Clang 23.1.5 both accept that source and emit one
# `vfmadd132ps`, so the 128-bit VEX forms belong to this target's contract --
# the 256-bit vectorizer ceiling being NONE for i686 is a codegen policy, not
# a licence to abort on a legal intrinsic.
#
# Why execution and not just "it compiles": a panic-to-no-op change would also
# make the compile succeed, and the second half of this gate's subject is the
# SA-11 operand placement, which only shows up in the VALUES. The probe is
# therefore built freestanding and static (no libc: this host has no 32-bit
# libc headers, and 12 bytes of `int $0x80` are all it needs) and RUN, so the
# numbers come from the i686 backend's own instructions.
#
# Two legs, and the second is what makes the first mean anything:
#   * every permutation's lanes equal a*b+c  (the fix)
#   * the PRE-FIX sequence is executed and is wrong (the probe can see it)
#
# Usage:
#   tests/regression/check_i686_fma128.sh [--lccc PATH]
#
# Assumes an x86-64 host, which is the only place a static i386 ELF runs
# natively; on any other host the run leg fails loudly rather than skipping,
# because a silently skipped gate is how this crash survived in the first place.

set -euo pipefail

LCCC=""
while [ $# -gt 0 ]; do
    case "$1" in
        --lccc) LCCC="$2"; shift 2 ;;
        -h|--help) sed -n '2,38p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "check_i686_fma128: unknown argument '$1'" >&2; exit 2 ;;
    esac
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC="$ROOT/tests/regression/i686_fma128.c"
PREFIX_SRC="$ROOT/tests/regression/i686_fma128_prefix.c"
CC="${LCCC:-$ROOT/target/fastbuild/lccc}"
for f in "$SRC" "$PREFIX_SRC"; do
    [ -f "$f" ] || { echo "FAIL: missing source $f" >&2; exit 1; }
done
[ -x "$CC" ] || { echo "FAIL: lccc binary '$CC' is not executable" >&2; exit 1; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# Host must be x86-64: an i386 static ELF executes natively there and nowhere
# else, and a cross-host SKIP would hide the regression this gate is for.
case "$(uname -m)" in
    x86_64|amd64) ;;
    *) echo "FAIL: this gate executes a 32-bit x86 binary and needs an x86-64 host (got $(uname -m))" >&2; exit 1 ;;
esac

echo "check_i686_fma128: i686 128-bit VEX FMA3 compile + placement"

# -mavx -mfma is the minimum that grants the VEX.128 encodings; -nostdlib and
# -ffreestanding because the probe talks to the kernel directly.
FLAGS="-m32 -O2 -mavx -mfma -ffreestanding -nostdlib -static -fno-pic"

# Leg 1: it must COMPILE. Before the fix this aborted with the panic above.
# shellcheck disable=SC2086 # FLAGS is a deliberate word list
if ! $CC $FLAGS "$SRC" -o "$WORK/probe" 2>"$WORK/cc.log"; then
    echo "FAIL: could not build the i686 128-bit FMA probe" >&2
    cat "$WORK/cc.log" >&2
    exit 1
fi
echo "  ok  the 128-bit VEX FMA spellings compile (they used to PANIC)"

# Leg 2: it must COMPUTE a*b+c, every lane of every permutation.
if ! out=$("$WORK/probe"); then
    echo "FAIL: the probe did not run: $out" >&2
    exit 1
fi
case "$out" in
    "PASS"*) echo "  ok  ${out#PASS }" ;;
    *) echo "FAIL: $out" >&2; exit 1 ;;
esac

# Leg 3: NON-VACUITY. The same harness, executing the operand placement the
# pre-fix code used (`vfmadd213ps %xmm2, %xmm0, %xmm1` then store `%xmm0`),
# must report wrong lanes. If it ever reports 0, the probe is measuring
# nothing and leg 2 is decoration.
# shellcheck disable=SC2086 # FLAGS is a deliberate word list
if ! $CC $FLAGS "$PREFIX_SRC" -o "$WORK/prefix" 2>"$WORK/cc2.log"; then
    echo "FAIL: could not build the pre-fix placement probe" >&2
    cat "$WORK/cc2.log" >&2
    exit 1
fi
if ! pout=$("$WORK/prefix"); then
    echo "FAIL: the pre-fix probe did not run: $pout" >&2
    exit 1
fi
case "$pout" in
    "0 of"*)
        echo "FAIL: the pre-fix placement reported 0 wrong lanes, so this gate" >&2
        echo "      cannot see the defect it exists for" >&2
        exit 1
        ;;
    *) echo "  ok  the pre-fix placement is detected: ${pout}" ;;
esac

echo "PASS: i686 128-bit FMA3 compiles and every permutation computes a*b+c"
