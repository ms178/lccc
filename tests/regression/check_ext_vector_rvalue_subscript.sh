#!/usr/bin/env bash
# Separate-TU vector rvalue indexing, both cross-compiler directions.
# --self-only is an explicit local smoke test, NOT cross-ABI validation.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$root/target/fastbuild/lccc}
[[ -x "$ccc" ]] || { echo "FAIL: compiler not built: $ccc" >&2; exit 1; }
compilers=("$ccc")
case ${1:-} in
    --self-only) ;;
    '')
        . "$root/tests/regression/lib/clang_oracle.sh"
        require_clang_oracle check_ext_vector_rvalue_subscript
        compilers+=("$CLANG_ORACLE_BIN") ;;
    *) echo "usage: $0 [--self-only]" >&2; exit 1 ;;
esac
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
src=$root/tests/regression/ext_vector_rvalue_subscript.c
# A freestanding i386 entry point avoids any multilib/libc prerequisite.
# Align the call frame, preserve main's status, and exit via the Linux ABI.
cat >"$work/start.s" <<'ASM'
.globl _start
_start:
    andl $-16, %esp
    call main
    movl %eax, %ebx
    movl $1, %eax
    int $0x80
.section .note.GNU-stack,"",@progbits
ASM
"$ccc" -m32 -c "$work/start.s" -o "$work/start.o"
for bits in 64 32; do
    link_flags=(-m"$bits")
    if [[ $bits == 32 ]]; then
        link_flags+=(-nostdlib -static "$work/start.o")
    fi
    for opt in -O0 -O1 -O2; do
        for p in "${!compilers[@]}"; do
            "${compilers[p]}" -m"$bits" "$opt" -DVECTOR_PROVIDER -c "$src" -o "$work/provider$p.o"
            "${compilers[p]}" -m"$bits" "$opt" -DVECTOR_CALLER -c "$src" -o "$work/caller$p.o"
        done
        for p in "${!compilers[@]}"; do
            for c in "${!compilers[@]}"; do
                "$ccc" "${link_flags[@]}" "$work/provider$p.o" "$work/caller$c.o" -o "$work/test"
                if ! "$work/test"; then
                    echo "FAIL: rvalue subscript m$bits $opt provider=$p caller=$c" >&2
                    exit 1
                fi
            done
        done
    done
done
echo "PASS: vector rvalue subscript (${#compilers[@]} compiler(s), separate TUs, -O0/-O1/-O2, m64/m32)"
