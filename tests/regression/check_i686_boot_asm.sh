#!/usr/bin/env bash
# i686 boot-path assembler pins: the kernel's .code32/.code16 compressed
# boot uses instructions the 64-bit path never sees.  Byte-compare against
# GAS (--32) — the boot firmware executes these bytes verbatim.
#
# Pins (regression: vmmcall/rdrand were "unhandled i686 instruction" and
# `.word . - sym - 1` died as "undefined label in .byte diff: .", all three
# in arch/x86/boot/compressed/mem_encrypt.o during the 6.18 bzImage build):
#   1. vmcall / vmmcall     — fixed 0F 01 C1 / 0F 01 D9
#   2. rdrand/rdseed reg    — 0F C7 /6|/7, 66 prefix on %16 destinations
#   3. `.word . - sym - 1`  — the location counter as a diff operand
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
case $CCC in /*) ;; *) CCC=$PWD/${CCC#./} ;; esac
tmp=${TMPDIR:-/tmp}/lccc-i686boot.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
cd "$tmp"
# The boot path is 32-bit code: the .code16 pins need the i686 driver
# (target selection is argv[0]-derived, so a same-binary link suffices).
ln -s "$CCC" "$tmp/i686-lccc"
CCC="$tmp/i686-lccc"

# Pin 1+2: instruction encodings byte-identical to GAS, including the
# .code16 operand-size inversion (in real mode %eax NEEDS the 66 prefix
# and %ax must NOT have it — the previous hand-rolled prefix logic got
# this backwards; the encoder now joins the central sized_op inversion).
printf '.code32\n.text\nvmmcall\nvmcall\nrdrand %%eax\nrdrand %%dx\nrdseed %%ebx\n.code16\nrdrand %%eax\nrdrand %%ax\nrdseed %%esi\n' > boot.s
"$CCC" -c -o boot_l.o boot.s
if command -v as >/dev/null; then
    as --32 -o boot_g.o boot.s
    cmp <(objdump -s -j .text boot_l.o | tail -n +4) \
        <(objdump -s -j .text boot_g.o | tail -n +4)
fi

# Pin 2b: register-class validation — byte/x87/MMX/XMM destinations are
# a hard error, not a silently wrong modrm reg field.
printf '.code32\nrdrand %%al\n' > bad1.s
if "$CCC" -c -o /dev/null bad1.s 2>/dev/null; then
    echo "FAIL: rdrand %al accepted" >&2; exit 1
fi
printf '.code32\nrdrand %%xmm0\n' > bad2.s
if "$CCC" -c -o /dev/null bad2.s 2>/dev/null; then
    echo "FAIL: rdrand %xmm0 accepted" >&2; exit 1
fi

# Pin 3: `.` as a diff operand (location counter at the directive).
printf '.text\nsym:\n nop\n .word . - sym - 1\n' > dot.s
"$CCC" -c -o dot_l.o dot.s
# sym is at 0, the .word is at offset 1: value must be 1 - 0 - 1 = 0.
objcopy -O binary --only-section=.text dot_l.o dot.bin
v=$(od -An -tu2 -j1 -N2 dot.bin | tr -d ' ')
[ "$v" = "0" ] || { echo "FAIL: .word . - sym - 1 = $v, want 0" >&2; exit 1; }

echo "i686 boot asm gate: PASS"
