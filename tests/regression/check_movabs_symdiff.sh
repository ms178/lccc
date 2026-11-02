#!/usr/bin/env bash
# Session-85 (Agent-Z audit, hunk #6): 64-bit symbol-difference mov immediate.
#
# arch/x86/mm/mem_encrypt_boot.S emits
#     movq $(.L__enc_copy_end - __enc_copy), %rcx
# Pristine main refused it: "symbol-difference mov immediate only supported
# at 32-bit width". The fix encodes a 64-bit mov immediate for the
# difference; both labels live in the same object, so the value folds to
# the absolute byte length.
#
# The self-referential length is NOT pinned to one encoding: GAS picks
# movabs imm64 (10 bytes -> length 11 with the `ret`), the integrated
# assembler picks the sign-extended imm32 form when the difference fits
# (7 bytes -> length 8). Both are legal fixpoints — the kernel idiom only
# needs the true byte count of the emitted code — so the expected value is
# derived from the labels themselves at runtime instead of hard-coding 11.
set -euo pipefail
CCC=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
tmp=${TMPDIR:-/tmp}/lccc-movabs.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

cat >"$tmp/len.s" <<'S'
	.text
	.globl	enc_copy_len
enc_copy_len:
	movq $(.Lend - enc_copy_len), %rax
	ret
.Lend:
	.globl	blob_start
blob_start:
	.byte 1,2,3,4,5,6,7,8,9,10,11,12,13
	.globl	blob_end
blob_end:
	.globl	blob_len
blob_len:
	movq $(blob_end - blob_start), %rax
	ret
	.globl	enc_copy_len_expected
enc_copy_len_expected:
	leaq .Lend(%rip), %rax
	leaq enc_copy_len(%rip), %rcx
	subq %rcx, %rax
	ret
S

cat >"$tmp/main.c" <<'C'
#include <stdio.h>
long enc_copy_len(void);
long enc_copy_len_expected(void);
long blob_len(void);
int main(void)
{
    long v = enc_copy_len();
    long e = enc_copy_len_expected();
    /* 8 (7-byte imm32 mov + ret) or 11 (10-byte movabs + ret). */
    if (e != 8 && e != 11) { printf("BAD expected %ld\n", e); return 3; }
    if (v != e) { printf("BAD len %ld expected %ld\n", v, e); return 1; }
    if (blob_len() != 13) { printf("BAD blob %ld\n", blob_len()); return 2; }
    printf("OK\n");
    return 0;
}
C

"$CCC" -c "$tmp/len.s" -o "$tmp/len.o"
"$CCC" -O2 "$tmp/main.c" "$tmp/len.o" -o "$tmp/t"
grep -q OK <<<"$("$tmp/t")"
