#!/usr/bin/env bash
# Session-04 (Cachymod 6.18.46 QEMU boot): `.code16` string ops must carry
# the correct operand-size override.
#
# Root cause: the i686 encoder emitted the bare opcode for `movsl`/`stosl`/
# `cmpsl`/`scasl`/`lodsl`/`insl`/`outsl` and never set sized_op, so the
# .code16 prefix-inversion fixup (which only rewrites instructions that made
# a size choice) left them prefix-less — in real mode that demotes them to
# WORD ops: `rep movsl` became `f3 a5` (movsw) instead of GAS's `66 f3 a5`,
# copying HALF the bytes. The kernel's arch/x86/boot/copy.S memcpy/memset
# then corrupted copy_boot_params and the BSS init, and the bzImage died in
# QEMU right after the linuxboot jump. The mirror bug applied to the explicit
# 16-bit forms (movsw/stosw/...) which push 0x66 without sized_op and thus
# kept the prefix under .code16, silently PROMOTING them to dword ops.
#
# Golden reference: GNU as (verified 2.44; re-verify against self-built 2.47
# when available) — `.code16` `rep movsl` = 66 f3 a5, `rep movsw` = f3 a5,
# `retl` = 66 c3.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
AS=${REFERENCE_AS:-as}
tmp=${TMPDIR:-/tmp}/lccc-code16str.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

cat >"$tmp/t.S" <<'ASM'
	.code16
	.text
	.globl t_memcpy
t_memcpy:
	pushw %si
	pushw %di
	shrw $2, %cx
	rep movsl
	popw %cx
	andw $3, %cx
	rep movsb
	popw %di
	popw %si
	retl
	.globl t_memset
t_memset:
	pushw %di
	movzbl %dl, %eax
	imull $0x01010101, %eax
	shrw $2, %cx
	rep stosl
	andw $3, %cx
	rep stosb
	popw %di
	retl
	.globl t_words
t_words:
	rep movsw
	rep stosw
	rep cmpsw
	retl
ASM

"$CCC" -m16 -c "$tmp/t.S" -o "$tmp/lccc.o"

# Byte-extract the .text content of each object and compare lccc vs the
# system/reference assembler.
extract() { # obj
  objdump -d "$1" | awk '/^[0-9a-f]+ </{f=1;next} f && /^[ \t]*[0-9a-f]+:/{for(i=2;i<=NF;i++){if($i ~ /^[0-9a-f][0-9a-f]$/) printf "%s ", $i; else break}; print ""}'
}
extract "$tmp/lccc.o" > "$tmp/lccc.bytes"

# Reference (GAS) output, if the assembler accepts the same input.
if "$AS" --32 "$tmp/t.S" -o "$tmp/gas.o" 2>/dev/null; then
  extract "$tmp/gas.o" > "$tmp/gas.bytes"
  if ! diff -u "$tmp/gas.bytes" "$tmp/lccc.bytes"; then
    echo "FAIL: .code16 string-op bytes diverge from GNU as" >&2
    exit 1
  fi
else
  echo "note: $AS unavailable or rejected input; using pinned expectations" >&2
fi

# Pinned golden bytes (GAS 2.44, 2026-08-26):
joined=$(tr -d ' \n' < "$tmp/lccc.bytes")
case "$joined" in
  # push si/di; shrw $2,cx; rep movsl (66 f3 a5); pop cx; andw $3,cx;
  # rep movsb; pop di/si; retl (66 c3); memset: push di; movzbl; imull
  # (66 69 c0 imm32); shrw; rep stosl (66 f3 ab); andw; rep stosb; pop di;
  # retl; words: f3 a5 (movsw), f3 ab (stosw), f3 a7 (cmpsw), retl
  5657c1e90266f3a55983e103f3a45f5e66c357660fb6c26669c001010101c1e90266f3ab83e103f3aa5f66c3f3a5f3abf3a766c3)
    ;;
  *)
    echo "FAIL: unexpected .code16 string-op encoding: $joined" >&2
    exit 1
    ;;
esac

echo "PASS: .code16 string ops match GAS (movsl=66 f3 a5, movsw=f3 a5, retl=66 c3)"

# ── Part 2: .code16gcc call/ret ABI (GCC -m16 convention) ──────────────
# GAS probed 2026-08-26 (GAS 2.44; re-verify against 2.47 when available):
# under .code16gcc UNSUFFIXED call/ret take 32-bit operands (66 e8 rel32 /
# 66 c3) while plain .code16 keeps 16-bit (e8 rel16 / c3). The boot code's
# hand asm (header.S calll main, copy.S retl, bioscall.S esp-relative arg
# reads + retl) assumes the GCC convention for lccc-compiled C; conflating
# the modes drifted the stack by 2 bytes per call and the boot died at the
# first function return (ret landed at 0x10000).
cat >"$tmp/abi.S" <<'ASM'
	.code16gcc
	.text
	.globl abi_a
abi_a:
	call abi_b
	ret
	.globl abi_b
abi_b:
	callw abi_a
	retw
	calll abi_a
	retl
	jmp abi_a
ASM

"$CCC" -m16 -c "$tmp/abi.S" -o "$tmp/abi-lccc.o"
raw() { objcopy -O binary -j .text "$1" /dev/stdout | od -An -v -tx1 | tr -d ' \n'; }
lccc_abi=$(raw "$tmp/abi-lccc.o")
gas_abi=""
if "$AS" --32 "$tmp/abi.S" -o "$tmp/abi-gas.o" 2>/dev/null; then
  gas_abi=$(raw "$tmp/abi-gas.o")
fi
# Pinned golden bytes at OBJECT level (GAS 2.44, 2026-08-26). Symbol-call
# displacement fields hold the relocation addend placeholder (fc ff ff ff /
# fe ff), exactly like GAS's unrelocated objects:
#   call abi_b     66 e8 fc ff ff ff      (rel32)
#   ret            66 c3
#   callw abi_a    e8 fe ff               (rel16)
#   retw           c3
#   calll abi_a    66 e8 fc ff ff ff      (rel32)
#   retl           66 c3
#   jmp abi_a      eb ea  (relaxed short form; full byte parity with GAS)
case "$lccc_abi" in
  66e8fcffffff66c3e8feffc366e8fcffffff66c3ebea)
    ;;
  *)
    echo "FAIL: .code16gcc call/ret ABI bytes: $lccc_abi" >&2
    exit 1
    ;;
esac
if [ -n "$gas_abi" ] && [ "$gas_abi" != "$lccc_abi" ]; then
  echo "FAIL: .code16gcc bytes diverge from GAS: lccc=$lccc_abi gas=$gas_abi" >&2
  exit 1
fi

# ── Part 3: lea operand-size overrides in .code16gcc ───────────────────
# `leal` must keep its 0x66 operand-size override (67 66 8d); dropping it
# truncated %esp-relative address computation to 16 bits and corrupted the
# boot CPUID probe (cpuid(1).eax stored through a garbage pointer ->
# "This kernel requires an x86-64 CPU, but only detected an i086 CPU").
cat >"$tmp/lea.S" <<'ASM'
	.code16gcc
	.text
	.globl lea_a
lea_a:
	leal 16(%esp), %eax
	leal 16(%esp), %ecx
	leal (%ebx), %eax
	leaw 16(%esp), %ax
	lea 16(%esp), %ax
	lea 16(%esp), %eax
ASM

"$CCC" -m16 -c "$tmp/lea.S" -o "$tmp/lea-lccc.o"
lccc_lea=$(raw "$tmp/lea-lccc.o")
case "$lccc_lea" in
  67668d44241067668d4c241067668d03678d442410678d44241067668d442410)
    ;;
  *)
    echo "FAIL: .code16gcc lea bytes: $lccc_lea" >&2
    exit 1
    ;;
esac
if "$AS" --32 "$tmp/lea.S" -o "$tmp/lea-gas.o" 2>/dev/null; then
  gas_lea=$(raw "$tmp/lea-gas.o")
  if [ "$gas_lea" != "$lccc_lea" ]; then
    echo "FAIL: lea bytes diverge from GAS: lccc=$lccc_lea gas=$gas_lea" >&2
    exit 1
  fi
fi

echo "PASS: .code16gcc ABI (unsuffixed call=66 e8 rel32, ret=66 c3; w/l suffixes honored; leal keeps 66)"

# ── Part 4: absolute-address operands use 16-bit addressing in .code16gcc ──
# Real mode defaults to 16-bit addressing. GAS encodes movs to/from a bare
# symbol as mod=00 rm=110 + disp16 (+ R_386_16), never mod=00 rm=101 (disp32)
# without a 0x67 override. A disp32-without-67 misdecodes in real mode: the
# CPU consumes only two address bytes (disp16), desyncing the instruction
# stream -- reproduced as a boot failure where setup's
# `movb $1, loaded_flags` ate bytes of the following call and corrupted
# get_cpuflags' stack frame (retl into 0x17290000).
cat > "$tmp/abs.S" <<'ASM'
	.code16gcc
	.text
	.globl abs_a
abs_a:
	movb $1, loaded_flags
	movl %eax, loaded_flags
	movzbl loaded_flags, %eax
	movb loaded_flags, %al
	movl $5, cpu_level
loaded_flags: .byte 0
cpu_level: .long 0
ASM

"$CCC" -m16 -c "$tmp/abs.S" -o "$tmp/abs-lccc.o"
lccc_abs=$(raw "$tmp/abs-lccc.o")
case "$lccc_abs" in
  c6061b000166a31b00660fb6061b00a01b0066c7061c00050000000000000000)
    ;;
  *)
    echo "FAIL: .code16gcc absolute-address mov bytes: $lccc_abs" >&2
    exit 1
    ;;
esac
if "$AS" --32 "$tmp/abs.S" -o "$tmp/abs-gas.o" 2>/dev/null; then
  gas_abs=$(raw "$tmp/abs-gas.o")
  if [ "$gas_abs" != "$lccc_abs" ]; then
    echo "FAIL: absolute-address bytes diverge from GAS: lccc=$lccc_abs gas=$gas_abs" >&2
    exit 1
  fi
fi

echo "PASS: .code16gcc absolute addressing (movb/movl/movzbl to bare symbols use disp16 + R_386_16, GAS-exact)"
