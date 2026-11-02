#!/usr/bin/env bash
# GAS mnemonics are case-insensitive: `CALL foo` == `call foo`, and the
# kernel's assembly mixes both (ftrace_64.S uses uppercase CALL via
# macros).  Branch mnemonics are special because a bare label operand is
# classified BY MNEMONIC (mnemonic_takes_label); a case-sensitive match
# there sent uppercase branches' labels down the generic-expression path
# ("unsupported call operand" / "unsupported jmp operand").
#
# Pins: uppercase branches assemble to byte-identical text sections as
# lowercase.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-asmcase.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

printf '.text\n.Lx:\n nop\n call .Lx\n jmp .Lx\n je .Lx\n loop .Lx\n' > "$tmp/lo.s"
printf '.text\n.Lx:\n nop\n CALL .Lx\n JMP .Lx\n JE .Lx\n LOOP .Lx\n' > "$tmp/up.s"

"$CCC" -c -o "$tmp/lo.o" "$tmp/lo.s"
"$CCC" -c -o "$tmp/up.o" "$tmp/up.s"

cmp <(objdump -s -j .text "$tmp/lo.o" | tail -n +4) \
    <(objdump -s -j .text "$tmp/up.o" | tail -n +4)

echo "asm case-insensitivity gate: PASS"
