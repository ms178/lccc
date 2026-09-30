#!/usr/bin/env bash
# NOTYPE-code PLT routing (S22) + NOTYPE-data `@PLTOFF` (S23): untyped
# (STT_NOTYPE, no .type) function exports must keep their PLT — a
# `lea f(%rip)` / `call f` / `movl $f` of untyped code must reach real
# code, never a copy of code bytes in BSS. GNU ld refuses the PIE
# `lea`-of-NOTYPE shape outright (TEXTREL error); lccc-ld keeps linking
# and stays correct. Conversely untyped DATA via `@PLTOFF` copy-relocates
# in executables (no JMP_SLOT) and is refused when linking -shared. The
# DSO and the callers are built by the host toolchain (not under test);
# only the link is lccc-ld's, via a gcc -B shim.
set -euo pipefail
CCC=${CCC:-target/fastbuild/lccc}
LD=${LD:-$(dirname "$CCC")/lccc-ld}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-notypecode.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/shim"
ln -s "$(cd "$(dirname "$LD")" && pwd)/$(basename "$LD")" "$tmp/shim/ld"
# Two untyped functions, deliberately without .type/.size (hand-written
# asm in the wild). Same source assembles for both architectures.
cat >"$tmp/nfn.s" <<'S'
        .text
        .globl notypefn
notypefn:
        movl $42, %eax
        ret
        .globl notypefn2
notypefn2:
        movl $7, %eax
        ret
S
# x86-64 caller: PC32 `lea` (address-of, canonical PLT) + PLT32 `call`.
cat >"$tmp/use64.s" <<'S'
        .text
        .globl main
main:
        pushq %rbp
        movq %rsp, %rbp
        pushq %rbx
        leaq notypefn(%rip), %rax
        call *%rax
        movl %eax, %ebx
        call notypefn2@plt
        movl %eax, %edx
        leaq .fmt(%rip), %rdi
        movl %ebx, %esi
        xorl %eax, %eax
        call printf@plt
        xorl %eax, %eax
        popq %rbx
        popq %rbp
        ret
        .section .rodata
.fmt:
        .string "%d %d\n"
S
# i386 caller (non-PIC: i686 lccc-ld is ET_EXEC-only): R_386_32
# address-of (canonical PLT) + PC32 direct call.
cat >"$tmp/use32.s" <<'S'
        .text
        .globl main
main:
        pushl %ebp
        movl %esp, %ebp
        pushl %ebx
        movl $notypefn, %eax
        call *%eax
        movl %eax, %ebx
        call notypefn2
        pushl %eax
        pushl %ebx
        pushl $.fmt
        call printf
        addl $12, %esp
        xorl %eax, %eax
        popl %ebx
        popl %ebp
        ret
        .section .rodata
.fmt:
        .string "%d %d\n"
S
expected="42 7"

# Host i386 capability, same three levels as check_copy_alias_sizes.sh:
# a host can link -m32 yet refuse to execute the result (no
# /lib/ld-linux.so.2, or a seccomp policy that SIGSYSes the ia32 gateway).
# Link success is not evidence the run leg can work; the PLT-routing
# relocation assertions below are execution-independent and run on every
# host that can link, so a sandboxed host still validates the S22 law.
i386_capability() {  # prints: run | link | none
    local rc=0
    if ! echo 'int main(void){return 0;}' \
        | gcc -m32 -x c -o "$tmp/i386probe" - >/dev/null 2>&1; then
        echo none
        return 0
    fi
    "$tmp/i386probe" >/dev/null 2>&1 || rc=$?
    if [[ $rc -eq 0 ]]; then echo run; else echo link; fi
}
i386_cap=$(i386_capability)

check_one() { # $1 = gcc -m flag or "", $2 = caller stem, $3 = extra link flags
    local m="$1" stem="$2" pie="$3"
    gcc $m -shared -fPIC -o "$tmp/libnfn.so" "$tmp/nfn.s"
    gcc $m -c "$tmp/$stem.s" -o "$tmp/$stem.o"
    # shellcheck disable=SC2086
    gcc $m $pie -B"$tmp/shim" "$tmp/$stem.o" -L"$tmp" -lnfn -o "$tmp/$stem"
    # Both callees PLT-routed (a JUMP_SLOT each), neither copied (a COPY
    # of code bytes into BSS would hand out an NX address). --use-dynamic:
    # the i386 output carries no section headers, so plain -r shows nothing.
    # Execution-independent: this is the S22 core law.
    local relocs
    relocs=$(readelf -rW --use-dynamic "$tmp/$stem")
    for fn in notypefn notypefn2; do
        grep -q "MP_SLOT.* $fn\( \|+\|$\)" <<<"$relocs" || {
            echo "FAIL${m:+ ($m)}: no JMP_SLOT for $fn" >&2; exit 1; }
        grep -q "COPY.* $fn\( \|+\|$\)" <<<"$relocs" && {
            echo "FAIL${m:+ ($m)}: stray COPY for $fn" >&2; exit 1; }
    done
    if [[ -z "$m" || "$i386_cap" == run ]]; then
        local got
        got=$(LD_LIBRARY_PATH="$tmp" "$tmp/$stem")
        if [[ "$got" != "$expected" ]]; then
            echo "FAIL${m:+ ($m)}: got '$got', expected '$expected'" >&2
            exit 1
        fi
    else
        echo "SKIP${m:+ ($m)}: host cannot execute i386; PLT-routing law still asserted"
    fi
    return 0
}
check_one "" use64 ""
if [[ "$i386_cap" != none ]]; then
    check_one -m32 use32 -no-pie
else
    echo "SKIP (-m32): host cannot link i386 at all"
fi
# x86-64 only: untyped DATA via `@PLTOFF` is data, not code — an
# executable copy-relocates it (its link-time address is the copy),
# while a shared link refuses it (a variable has no PLT entry).
cat >"$tmp/nfd.s" <<'S'
        .data
        .globl notypedata
        .size notypedata, 8
notypedata:
        .quad 0x1122334455667788
S
cat >"$tmp/usepf.s" <<'S'
        .text
        .globl main
main:
        pushq %rbp
        movq %rsp, %rbp
        leaq _GLOBAL_OFFSET_TABLE_(%rip), %rcx
        movabsq $notypedata@PLTOFF, %rax
        addq %rcx, %rax
        movq (%rax), %rsi
        leaq .fmt(%rip), %rdi
        xorl %eax, %eax
        call printf@plt
        xorl %eax, %eax
        popq %rbp
        ret
        .section .rodata
.fmt:
        .string "%lx\n"
S
check_pltoff() {
    gcc -shared -fPIC -o "$tmp/libnfd.so" "$tmp/nfd.s"
    gcc -c "$tmp/usepf.s" -o "$tmp/usepf.o"
    local mode
    for mode in -no-pie -pie; do
        # shellcheck disable=SC2086
        gcc $mode -B"$tmp/shim" "$tmp/usepf.o" -L"$tmp" -lnfd -o "$tmp/usepf"
        local got
        got=$(LD_LIBRARY_PATH="$tmp" "$tmp/usepf")
        if [[ "$got" != "1122334455667788" ]]; then
            echo "FAIL (pltoff $mode): got '$got'" >&2; exit 1
        fi
        local relocs
        relocs=$(readelf -rW --use-dynamic "$tmp/usepf")
        grep -q "COPY.* notypedata\( \|+\|$\)" <<<"$relocs" || {
            echo "FAIL (pltoff $mode): no COPY for notypedata" >&2; exit 1; }
        grep -q "MP_SLOT.* notypedata\( \|+\|$\)" <<<"$relocs" && {
            echo "FAIL (pltoff $mode): stray JMP_SLOT for notypedata" >&2; exit 1; }
    done
    if gcc -shared -B"$tmp/shim" "$tmp/usepf.o" -L"$tmp" -lnfd \
        -o "$tmp/pf.so" 2>"$tmp/pf.err"; then
        echo "FAIL (pltoff): -shared link unexpectedly succeeded" >&2; exit 1
    fi
    grep -q "R_X86_64_PLTOFF64 against shared-library variable 'notypedata'" \
        "$tmp/pf.err" || {
        echo "FAIL (pltoff): wrong -shared diagnostic:" >&2
        cat "$tmp/pf.err" >&2; exit 1; }
    return 0
}
check_pltoff
echo "PASS: notype-code PLT routing (x86-64 + i386) + notype-data PLTOFF (x86-64)"
