#!/usr/bin/env bash
# ============================================================================
# kernel_boot_isa.sh — the ISA contract of the kernel boot path.
#
# WHY
# ---
# Two kbuild files in the pre-boot path REPLACE KBUILD_CFLAGS instead of
# extending it, and a replacement also discards the `-march=` that
# arch/x86/Makefile derives from CONFIG_GENERIC_CPU / CONFIG_X86_64_VERSION:
#
#   arch/x86/boot/compressed/Makefile:27     KBUILD_CFLAGS := -m$(BITS) -O2 ...
#   drivers/firmware/efi/libstub/Makefile:38 KBUILD_CFLAGS := $(cflags-y) ...
#
# For GCC, Clang and ICC that is harmless: their absent-`-march` default IS the
# x86-64 baseline.  LCCC's documented default ISA is the integer-v3 profile
# ("no explicit -march: project baseline x86-64-v3", src/driver/pipeline.rs),
# so the same Makefile silently built a BMI2 decompressor.  Measured on QEMU's
# `qemu64` model: `shrxq` in kernel_ident_mapping_init -> #UD -> triple fault,
# and a triple fault in early 64-bit startup prints nothing on the serial
# console at all: it looks exactly like "the compiler miscompiles everything".
#
# The kernel names the ISA everywhere else in this path — REALMODE_CFLAGS
# carries `-march=i386`, and both flag-wiping files carry `-march=i386` for
# their CONFIG_X86_32 branch — so naming it in the x86-64 branch is the same
# idiom, and for GCC/Clang/ICC the flag changes nothing (explicitly requesting
# the baseline they already default to).  One compiler-agnostic line, GCC-exact
# flag sets, no kernel source semantics touched.
#
# WHAT
# ----
#   patch <kernel-dir>   make the two files name the x86-64 baseline ISA
#                        (idempotent; a tree carrying NEITHER the pristine nor
#                        the patched line is an error, never a silent skip)
#   check <kernel-dir>   assert only (no writes): both branches of both files
#                        name their ISA
#   scan <path>...       audit built boot objects (*.o in each file/directory
#                        given) for instructions above the x86-64 baseline
#
# `scan` is the measured half of the contract: `patch` fixes the flags, `scan`
# proves the artifacts.  It is a per-object scan on purpose — the linked stage-2
# image contains the compressed payload, where a data word could look like an
# instruction.
# ============================================================================
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

# The BMI/ABM class, the VEX/AVX class and the SSE4.2 class, verbatim from
# tests/regression/check_isa_gate_escape.sh: a boot path that claims the
# x86-64 baseline may contain none of them (the kernel's minimum CPU for
# CONFIG_GENERIC_CPU has SSE2 and nothing above it).
BMI_RE='\b(rorx[ql]|andn[ql]?|shrx[ql]|shlx[ql]|sarx[ql]|mulx[ql]|bzhi[ql]|bextr[ql]?|blsi[ql]?|blsmsk[ql]?|blsr[ql]?|pdep[ql]?|pext[ql]?|tzcnt[ql]?|lzcnt[ql]?|movbe[ql]?)\b'
VEX_RE='(^|[[:space:]])v[a-z][a-z0-9]+[[:space:]]'
SSE42_RE='\b(pcmpgtq|pcmpistri|pcmpestri|crc32[ql]?|popcnt[ql]?)\b'
ISA_RE="$BMI_RE|$VEX_RE|$SSE42_RE"

# `verr` and `verw` are the only two REAL x86 mnemonics that begin with `v`.
# They are 286 instructions and part of the x86-64 baseline, so they are not
# VEX evidence.  They matter here because objdump decodes a .code16 object
# (arch/x86/boot is -m16) with its default 32/64-bit algorithm: the
# instruction/data boundary errors that produces invent plausible-looking
# mnemonics out of operand bytes, and the VEX pattern's `v[a-z]+` net caught
# `verr (%ebx)` out of the middle of `movzww`'s operand in video-vesa.o — a
# FAIL that was entirely the scanner's own.  Deleting the two legacy
# mnemonics first keeps the net as wide as before (any other `v*` mnemonic is
# an AVX encoding and must not be in a baseline build) without the two
# false positives.
LEGACY_V_RE='(^|[[:space:]])(verr|verw)([[:space:]]|$)'

# file / pristine line / patched line / description
FILES=(
    "arch/x86/boot/compressed/Makefile"
    "drivers/firmware/efi/libstub/Makefile"
)
PRISTINE=(
    'cflags-$(CONFIG_X86_64) := -mcmodel=small -mno-red-zone'
    $'cflags-$(CONFIG_X86_64)\t\t:= -mcmodel=small'
)
PATCHED=(
    'cflags-$(CONFIG_X86_64) := -march=x86-64 -mcmodel=small -mno-red-zone'
    $'cflags-$(CONFIG_X86_64)\t\t:= -march=x86-64 -mcmodel=small'
)
WHAT=(
    'decompressor (x86-64)'
    'EFI stub (x86-64)'
)
# The 32-bit branch of both files must keep naming its ISA: that symmetry is
# the whole argument for naming it on the 64-bit branch.
X86_32_LINE=(
    'cflags-$(CONFIG_X86_32) := -march=i386'
    $'cflags-$(CONFIG_X86_32)\t\t:= -march=i386'
)

usage() {
    sed -n '3,45p' "$0" | sed 's/^# \{0,1\}//'
    exit 2
}

# --- patch ------------------------------------------------------------------
do_patch() {
    local kdir=$1 i rc=0 file old new what n32
    for i in "${!FILES[@]}"; do
        file="$kdir/${FILES[$i]}"
        old=${PRISTINE[$i]}
        new=${PATCHED[$i]}
        what=${WHAT[$i]}
        n32=${X86_32_LINE[$i]}
        if [[ ! -f $file ]]; then
            echo "kernel_boot_isa: missing $file" >&2
            rc=1
            continue
        fi
        # The 32-bit branch is the reference: if it stopped naming its ISA the
        # file was refactored and this patch must be re-derived, not guessed.
        if ! grep -qF -- "$n32" "$file"; then
            echo "kernel_boot_isa: $what: expected '$n32' in ${FILES[$i]} — the flag" >&2
            echo "  layout changed upstream; re-derive the x86-64 line instead of guessing." >&2
            rc=1
            continue
        fi
        if grep -qF -- "$new" "$file"; then
            echo "kernel_boot_isa: $what already explicit"
            continue
        fi
        if ! grep -qF -- "$old" "$file"; then
            echo "kernel_boot_isa: $what: neither the pristine nor the patched line is" >&2
            echo "  present in ${FILES[$i]} (expected '$old')." >&2
            rc=1
            continue
        fi
        # Anchor both ends: a longer line that merely contains the pristine
        # text (a continuation, a comment) must not be rewritten.
        local esc_old esc_new
        esc_old=$(printf '%s' "$old" | sed 's/[.*[\^$]/\\&/g')
        esc_new=$(printf '%s' "$new" | sed 's/[&\\]/\\&/g')
        sed -i "s|^${esc_old}\$|${esc_new}|" "$file"
        if ! grep -qF -- "$new" "$file"; then
            echo "kernel_boot_isa: $what: rewrite of ${FILES[$i]} failed" >&2
            rc=1
            continue
        fi
        if grep -qF -- "$old" "$file"; then
            echo "kernel_boot_isa: $what: pristine line still present after rewrite" >&2
            rc=1
            continue
        fi
        echo "kernel_boot_isa: $what -> $new"
    done
    return $rc
}

# --- check ------------------------------------------------------------------
do_check() {
    local kdir=$1 i rc=0 file n32 wanted
    for i in "${!FILES[@]}"; do
        file="$kdir/${FILES[$i]}"
        n32=${X86_32_LINE[$i]}
        wanted=${PATCHED[$i]}
        if [[ ! -f $file ]]; then
            echo "kernel_boot_isa: FAIL: missing $file" >&2
            rc=1
            continue
        fi
        if ! grep -qF -- "$n32" "$file"; then
            echo "kernel_boot_isa: FAIL: ${FILES[$i]}: x86-32 branch lost '$n32'" >&2
            rc=1
        fi
        if ! grep -qF -- "$wanted" "$file"; then
            echo "kernel_boot_isa: FAIL: ${FILES[$i]}: x86-64 branch does not name its ISA" >&2
            echo "  expected: $wanted" >&2
            echo "  run: $SCRIPT_DIR/kernel_boot_isa.sh patch <kernel-dir>" >&2
            rc=1
        fi
    done
    [[ $rc -eq 0 ]] && echo "kernel_boot_isa: check PASS (both branches of both files name their ISA)"
    return $rc
}

# --- scan -------------------------------------------------------------------
scan_obj() { # scan_obj <file> ; echoes the offending mnemonics, rc 1 on a hit
    local file=$1 hits
    # Strip `#` comments first: a comment naming an instruction is not one.
    hits=$(objdump -d --no-show-raw-insn "$file" 2>/dev/null | sed 's/#.*//' \
           | sed -E "s/$LEGACY_V_RE//g" \
           | grep -oE "$ISA_RE" | sort -u | tr '\n' ' ' || true)
    if [[ -n ${hits// /} ]]; then
        printf 'kernel_boot_isa: FAIL: %s: %s\n' "$file" "$hits" >&2
        return 1
    fi
    return 0
}

do_scan() {
    local rc=0 n=0 path f
    command -v objdump >/dev/null 2>&1 || {
        echo "kernel_boot_isa: objdump not found (binutils is a build-machine tool)" >&2
        return 2
    }
    for path in "$@"; do
        if [[ -d $path ]]; then
            for f in "$path"/*.o; do
                [[ -f $f ]] || continue
                n=$((n + 1))
                scan_obj "$f" || rc=1
            done
        elif [[ -f $path ]]; then
            n=$((n + 1))
            scan_obj "$path" || rc=1
        else
            echo "kernel_boot_isa: scan: no such file or directory: $path" >&2
            rc=1
        fi
    done
    if [[ $n -eq 0 ]]; then
        echo "kernel_boot_isa: scan: no objects found in: $*" >&2
        return 1
    fi
    if [[ $rc -eq 0 ]]; then
        echo "kernel_boot_isa: scan PASS ($n object(s), 0 BMI/ABM/VEX/SSE4.2-class instructions)"
    fi
    return $rc
}

case ${1:-} in
    patch) [[ $# -ge 2 ]] || usage; do_patch "$2" ;;
    check) [[ $# -ge 2 ]] || usage; do_check "$2" ;;
    scan)  [[ $# -ge 2 ]] || usage; shift; do_scan "$@" ;;
    *)     usage ;;
esac
