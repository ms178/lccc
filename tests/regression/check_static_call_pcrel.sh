#!/usr/bin/env bash
# Session-85 (Agent-Z audit, hunk #7): `.long sym - (. + 4)` (static_call
# trampoline shape) must produce a PC32 diff relocation, not a silent zero.
#
# Reproduced on pristine main: assembled to 00000000 with NO relocation
# (GAS: R_X86_64_PC32 target-4). Silent misassembly is the worst failure
# class. The runtime check reconstructs dest = site_end + stored_rel exactly
# like arch/x86/kernel/static_call.c's insn_decode and compares pointers.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-static-call.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

cat >"$tmp/site.s" <<'S'
	.text
	.globl	target_fn
target_fn:
	xorl	%eax, %eax
	ret
	.section .sc_sites,"a"
	.globl	site_start
	.globl	site_end
site_start:
	.long	target_fn - (. + 4)
site_end:
S

cat >"$tmp/main.c" <<'C'
#include <stdio.h>
extern char site_start[], site_end[];
extern int target_fn(int);

int main(void)
{
    unsigned int *site = (unsigned int *)site_start;
    int rel = (int)*site;
    char *dest = (char *)site_end + rel;   /* site_end + rel */
    if (dest != (char *)target_fn) {
        printf("BAD: site=%p rel=%d dest=%p target=%p\n",
               (void *)site, rel, (void *)dest, (void *)target_fn);
        return 1;
    }
    if ((long)(site_end - site_start) != 4) return 2;
    printf("OK\n");
    return target_fn(0) != 0;
}
C

"$CCC" -c "$tmp/site.s" -o "$tmp/site.o"
# Object-level guard: the slot must NOT be all-zero-without-relocation.
if command -v objdump >/dev/null 2>&1; then
    objdump -r "$tmp/site.o" | grep -q "site_start\|target_fn" || {
        echo "no relocation recorded for the static_call site" >&2
        exit 1
    }
fi
"$CCC" -O2 "$tmp/main.c" "$tmp/site.o" -o "$tmp/t"
"$tmp/t" | grep -q OK
