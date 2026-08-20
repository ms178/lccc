#!/usr/bin/env bash
# Structural regression for real-mode branch relaxation.
#
# The i686 encoder correctly reports 4/3-byte rel16 Jcc/jmp instructions, but
# the shared ELF writer once re-checked them against hard-coded 6/5-byte rel32
# lengths and silently omitted them from relaxation.  Every local branch in
# Linux arch/x86/boot then stayed near, wasting roughly 900 bytes in setup.elf.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-code16-relax.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

cat >"$tmp/probe.s" <<'ASM'
.code16gcc
.text
.globl code16_relax_probe
.type code16_relax_probe, @function
code16_relax_probe:
        xorl    %eax, %eax
        je      .Lshort_cond
        jmp     .Lshort_jump
.Lshort_cond:
        incl    %eax
.Lshort_jump:
        # This target is deliberately outside rel8 range.  The writer starts
        # optimistically short, then must restore the original rel16 form.
        jne     .Lfar
        .fill   128, 1, 0x90
.Lfar:
        jmp     .Lshort_jump
        retl
.size code16_relax_probe, .-code16_relax_probe
ASM

"$CCC" -m32 -c "$tmp/probe.s" -o "$tmp/lccc.o"
as --32 "$tmp/probe.s" -o "$tmp/gas.o"
objcopy -O binary --only-section=.text "$tmp/lccc.o" "$tmp/lccc.text"
objcopy -O binary --only-section=.text "$tmp/gas.o" "$tmp/gas.text"

if ! cmp -s "$tmp/lccc.text" "$tmp/gas.text"; then
    echo "code16 relaxation differs from GNU as" >&2
    echo "LCCC: $(od -An -tx1 -v "$tmp/lccc.text" | tr -d ' \n')" >&2
    echo "GAS:  $(od -An -tx1 -v "$tmp/gas.text" | tr -d ' \n')" >&2
    exit 1
fi

# Prove the comparison did not accidentally accept an all-near stream: the
# first Jcc/Jmp must be short while the deliberately distant Jcc stays rel16.
hex=$(od -An -tx1 -v "$tmp/lccc.text" | tr -d ' \n')
[[ $hex == 6631c07402eb02* ]] || {
    echo "expected short code16 Jcc/Jmp prefix, got $hex" >&2
    exit 1
}
[[ $hex == *0f85* ]] || {
    echo "expected an out-of-range rel16 Jcc, got $hex" >&2
    exit 1
}
