#!/usr/bin/env bash
# Session-85 (Agent-Z audit, hunk #6): 64-bit symbol-difference mov immediate.
#
# arch/x86/mm/mem_encrypt_boot.S emits
#     movq $(.L__enc_copy_end - __enc_copy), %rcx
# Pristine main refused it: "symbol-difference mov immediate only supported
# at 32-bit width". The fix encodes movabs (REX.W B8+rd) with an R_X86_64_64
# diff relocation; both labels live in the same object, so the link folds the
# difference to the absolute byte length.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
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
S

cat >"$tmp/main.c" <<'C'
#include <stdio.h>
long enc_copy_len(void);
long blob_len(void);
int main(void)
{
    /* .Lend sits right after the `ret`: movabs imm64 is exactly 10 bytes
       (REX.W + B8+rd + 8-byte immediate) and ret is 1 → 11. */
    long v = enc_copy_len();
    if (v != 11) { printf("BAD len %ld\n", v); return 1; }
    if (blob_len() != 13) { printf("BAD blob %ld\n", blob_len()); return 2; }
    printf("OK\n");
    return 0;
}
C

"$CCC" -c "$tmp/len.s" -o "$tmp/len.o"
"$CCC" -O2 "$tmp/main.c" "$tmp/len.o" -o "$tmp/t"
"$tmp/t" | grep -q OK
