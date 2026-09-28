#!/usr/bin/env bash
# D32 AArch64 width gate: _Decimal32 traffic moves through w registers.
#
# AArch64 has no 32-bit DImode move distinct from the integer one, so D32
# rides the U32 move tables (`str_for_type`, `reg_for_type`,
# `load_instr_for_type_impl`) and the indexed emitter's w-view arms: a
# 32-bit carrier is a 32-bit carrier — moves never interpret the bits.
# Pins the contract structurally on the `-S` output:
#   * the indexed store folds into `str wN, [xB, xI, lsl #2]` (folded AND
#     width-exact — the fold used to be refused because the emitter only
#     had 64-bit arms);
#   * plain (non-indexed) array loads/stores are `ldr w0`/`str w0` (4-byte
#     exact — they used to over-read/over-store through the x view,
#     clobbering the neighboring packed element);
#   * the ONLY x-view memory traffic left is `[sp, #N]` slot spills, which
#     are full-8-byte-slot correct (D32 spill slots are 8 bytes);
#   * no bare `mov xN/wN, #M` outside the encodable imm16 window (M > 65535
#     or M < -65536): GAS rejects those, so large D32 constants must
#     materialize through movz/movk (S19/F1 encodability teeth).
# Structural-only: this CI has no aarch64 execution environment (no
# qemu-aarch64, no cross toolchain), so there is no differential half.
# Value-correctness of the same recurrence shape is carried by the x86-64
# D64 gate's gcc differential plus the `d32_*` unit tests pinning the exact
# emission strings; a cross-assemble step would add nothing grep cannot see
# (every mnemonic/register pair asserted here is also emitted for U32,
# which assembles today).
# Hermetic (S19/F2): the test C file declares printf manually and this
# script passes zero -I flags — no host headers, identical -S output.
set -eu

CCC=${CCC_ARM:-./target/fastbuild/lccc-arm}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tmp="${TMPDIR:-/tmp}/lccc-d32-arm-fold.$$"
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
mkdir -p "$tmp"

"$CCC" -O2 -S "$dir/decimal32_indexed_fold_arm.c" -o "$tmp/out.s"

# Folded store: `str wN, [xB, xI, lsl #2]`.
if ! grep -qE 'str w[0-9]+, \[x[0-9]+, x[0-9]+, lsl #2\]' "$tmp/out.s"; then
    echo "FAIL: no folded w-view D32 store in $tmp/out.s" >&2
    grep -E 'str ' "$tmp/out.s" >&2 | head -10
    exit 1
fi
# Plain array load through the w view (the T3 half of the fix).
if ! grep -qE 'ldr w0, \[x[0-9]+\]' "$tmp/out.s"; then
    echo "FAIL: no w-view plain D32 load in $tmp/out.s" >&2
    grep -E 'ldr ' "$tmp/out.s" >&2 | head -10
    exit 1
fi
# Plain array store through the w view (the T1+T2 half of the fix).
if ! grep -qE 'str w0, \[x[0-9]+\]' "$tmp/out.s"; then
    echo "FAIL: no w-view plain D32 store in $tmp/out.s" >&2
    grep -E 'str ' "$tmp/out.s" >&2 | head -10
    exit 1
fi
# No 64-bit indexed traffic anywhere: this file's only arrays are the two
# _Decimal32 tables, so any SIB-scale x-view memory op is an over-access.
if grep -qE '(ldr|str) x[0-9]+, \[x[0-9]+, x[0-9]+, lsl' "$tmp/out.s"; then
    echo "FAIL: 64-bit indexed memory traffic in $tmp/out.s" >&2
    grep -E '(ldr|str) x[0-9]+, \[x[0-9]+, x[0-9]+, lsl' "$tmp/out.s" >&2 | head -10
    exit 1
fi
# No x-view memory traffic through a non-sp base at all: every remaining
# x-view memory op must be an 8-byte slot spill (`[sp, #N]`), which is
# full-slot correct. Any `x0`-through-`[xN]` op is a packed-array
# over-read/over-store.
if grep -qE '(ldr|str) x[0-9]+, \[x' "$tmp/out.s"; then
    echo "FAIL: x-view array/global traffic in $tmp/out.s" >&2
    grep -E '(ldr|str) x[0-9]+, \[x' "$tmp/out.s" >&2 | head -10
    exit 1
fi
# Encodability teeth (S19/F1+F3): a bare `mov xN/wN, #M` is only encodable
# inside the single-mov (movz-alias) window -65536 <= M <= 65535.
# Garden-variety decimal literals all exceed it (2.0DF = 0x32000014) and
# GAS rejects the bare form past imm16. movz/movk/movn carry imm16 by
# construction and cannot match the `mov [xw]` shape below.
if grep -oE 'mov [xw][0-9]+, #-?[0-9]+' "$tmp/out.s" \
    | sed -E 's/^mov [xw][0-9]+, #(-?[0-9]+)$/\1/' \
    | awk '$1 > 65535 || $1 < -65536 { found = 1 } END { exit found ? 0 : 1 }'; then
    echo "FAIL: bare mov outside the encodable imm16 window in $tmp/out.s" >&2
    grep -E 'mov [xw][0-9]+, #' "$tmp/out.s" >&2 | head -10
    exit 1
fi
echo "PASS: D32 AArch64 width (folded str wN SIB, w0 plain traffic, x-view is spills only)"
