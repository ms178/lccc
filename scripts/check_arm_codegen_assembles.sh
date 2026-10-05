#!/usr/bin/env bash
# Assemble LCCC AArch64 -O2 codegen with the integrated assembler, and when
# GNU as is present byte-compare .text against it.
#
# The word-level encoder oracle cannot see directive / layout bugs (the
# `.p2align N,,M` max-padding miss shifted every later branch target). This
# gate compiles every tests/regression/arm_*.c file to assembly, assembles
# that assembly with lccc-arm, and — if aarch64-linux-gnu-as + objcopy are
# on PATH — also assembles it with GAS and cmp's the two .text images.
#
# Missing cross-binutils is SKIP, not PASS: the integrated-assembler half
# still runs. A host without lccc-arm is SKIP (the build gate produces it).
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cc=${CCC_ARM:-$repo/target/fastbuild/lccc-arm}
inc=${GCC_INC:-$(gcc -print-file-name=include)}
gas=$(command -v aarch64-linux-gnu-as || true)
objcopy=$(command -v aarch64-linux-gnu-objcopy || true)

if [[ ! -x "$cc" ]]; then
    echo "SKIP  arm-codegen-assembler-parity (no $cc)"
    exit 0
fi

shopt -s nullglob
files=("$repo"/tests/regression/arm_*.c)
if [[ ${#files[@]} -eq 0 ]]; then
    echo "FAIL: no tests/regression/arm_*.c files" >&2
    exit 1
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

assembled=0
compared=0
skipped_pp=0
for src in "${files[@]}"; do
    base=$(basename "$src" .c)
    if ! "$cc" -O2 -I"$inc" -S "$src" -o "$tmp/$base.s" 2>"$tmp/$base.pp.err"; then
        # Host preprocessor / missing headers: skip this file, do not fail
        # the gate.  The encoder bugs this gate exists for only show up
        # after a successful compile.
        skipped_pp=$((skipped_pp + 1))
        continue
    fi
    if ! "$cc" -c "$tmp/$base.s" -o "$tmp/$base.lccc.o" 2>"$tmp/$base.as.err"; then
        echo "FAIL: integrated assembler rejected $base.s" >&2
        cat "$tmp/$base.as.err" >&2
        exit 1
    fi
    assembled=$((assembled + 1))
    if [[ -n "$gas" && -n "$objcopy" ]]; then
        "$gas" -o "$tmp/$base.gas.o" "$tmp/$base.s"
        "$objcopy" -O binary -j .text "$tmp/$base.lccc.o" "$tmp/$base.lccc.bin"
        "$objcopy" -O binary -j .text "$tmp/$base.gas.o" "$tmp/$base.gas.bin"
        if ! cmp -s "$tmp/$base.lccc.bin" "$tmp/$base.gas.bin"; then
            echo "FAIL: .text differs from GNU as for $base" >&2
            wc -c "$tmp/$base.lccc.bin" "$tmp/$base.gas.bin" >&2
            exit 1
        fi
        compared=$((compared + 1))
    fi
done

echo "arm-codegen-assembler-parity: assembled $assembled, gas-compared $compared, preprocessor-skip $skipped_pp"
if [[ "$assembled" -eq 0 ]]; then
    echo "FAIL: no ARM regression file produced assembly" >&2
    exit 1
fi
