#!/usr/bin/env bash
# GOT64 local slots against the PRE-2.44 assembler spelling (S24): three
# locals at `.data` offsets 0/16/32, each loaded through a synthetic
# `R_X86_64_GOT64 .data+N` (section symbol + addend) and compared against
# its own address. The slot must HOLD `S+A` (one slot per address); folding
# the addend into the field reads past the slot and answers wrong.
#
# Modern GAS (>= 2.44) spells `$lvar@GOT` as `R_X86_64_GOT64 lvar+0` and
# rejects `.data+N@GOT` outright, so the old spelling is unproducible
# through the sugar -- hence the `.reloc` directive, which emits it
# exactly (verified: `R_X86_64_GOT64 .data + N`). The `. - 8` is relative
# to the `movabs` immediate, so no byte counting can rot. The gate asserts
# the input spelling first: if a future assembler ever stops honouring
# `.reloc`, the gate fails loudly instead of testing the wrong thing.
#
# DELIBERATE bfd divergence, pinned here: GNU ld 2.44 answers `1 0 0` on
# this input (it folds the addend into the field and reads past the slot).
# lccc-ld answers `1 1 1`: matching the reference linker here would mean
# matching its miscompile. Both link modes are covered.
set -euo pipefail
CCC=${CCC:-target/fastbuild/lccc}
LD=${LD:-$(dirname "$CCC")/lccc-ld}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-got64old.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/shim"
ln -s "$(cd "$(dirname "$LD")" && pwd)/$(basename "$LD")" "$tmp/shim/ld"
cat >"$tmp/t.s" <<'S'
	.text
	.macro check n, off
	.globl check\n
	.type check\n, @function
check\n:
	leaq _GLOBAL_OFFSET_TABLE_(%rip), %rcx
	movabsq $0, %rax
	.reloc . - 8, R_X86_64_GOT64, .data+\off
	movq (%rcx,%rax), %rax
	leaq lvar\n(%rip), %rcx
	cmpq %rcx, %rax
	sete %al
	movzbl %al, %eax
	ret
	.size check\n, .-check\n
	.endm
	check 0, 0
	check 1, 16
	check 2, 32
	.globl main
	.type main, @function
main:
	pushq %rbp
	movq %rsp, %rbp
	pushq %rbx
	pushq %r12
	pushq %r13
	call check0
	movl %eax, %ebx
	call check1
	movl %eax, %r12d
	call check2
	movl %eax, %r13d
	leaq fmt(%rip), %rdi
	movl %ebx, %esi
	movl %r12d, %edx
	movl %r13d, %ecx
	xorl %eax, %eax
	call printf@PLT
	xorl %eax, %eax
	popq %r13
	popq %r12
	popq %rbx
	popq %rbp
	ret
	.size main, .-main
	.section .rodata
fmt:	.string "%d %d %d\n"
	.data
lvar0:	.quad 0
	.skip 8
lvar1:	.quad 0
	.skip 8
lvar2:	.quad 0
S
as "$tmp/t.s" -o "$tmp/t.o"
# The fixture IS the old spelling: three GOT64 relocs against `.data`.
n=$(readelf -rW "$tmp/t.o" | grep -c "R_X86_64_GOT64.*\.data + ")
[ "$n" = "3" ] || { echo "fixture lost its old-spelling relocs (found $n, want 3)"; exit 1; }
for mode in -no-pie -pie; do
  gcc -B "$tmp/shim" "$tmp/t.o" -o "$tmp/a-$mode" "$mode"
  out=$("$tmp/a-$mode")
  [ "$out" = "1 1 1" ] || { echo "$mode: got '$out', want '1 1 1'"; exit 1; }
done
echo "got64-old-spelling: 1 1 1 in -no-pie and -pie"
