#!/usr/bin/env bash
# Freestanding cross-TU vector ABI witnesses: no 32-bit libc installation needed.
set -euo pipefail
cd "$(dirname "$0")/../.."
source tests/regression/lib/gcc_oracle.sh
source tests/regression/lib/clang_oracle.sh
require_gcc_oracle i686-gnu-vector-abi
require_clang_oracle i686-gnu-vector-abi
[[ $("$GCC_ORACLE_BIN" -dumpfullversion) == 16.2.0 ]] || {
    echo 'FAIL: this gate requires GCC 16.2.0 exactly' >&2; exit 1;
}
ccc=${CCC:-target/fastbuild/lccc}
linker=${LCCC_LD:-$(dirname "$ccc")/lccc-ld}
assembler=${LCCC_TEST_ASSEMBLER:-as}
[[ $("$assembler" --version) == *'2.47'* ]] || {
    echo 'FAIL: this gate requires GNU as 2.47' >&2; exit 1;
}
as_dir=$(dirname "$(command -v "$assembler")")
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fixture=tests/regression/fixtures/i686-gnu-vector-abi
base_flags=(-m32 -fno-pic -fno-pie -fno-stack-protector -Wno-psabi)
cat > "$tmp/start.s" <<'ASM'
.text
.globl _start
_start:
    andl $-16, %esp
    call main
    movl %eax, %ebx
    movl $1, %eax
    int $0x80
.globl puts
puts:
    xorl %eax, %eax
    ret
.section .note.GNU-stack,"",@progbits
ASM
"$assembler" --32 "$tmp/start.s" -o "$tmp/start.o"
# The typed source is shared with the x86-64 fixture. Only replace libc headers
# with declarations; keep the checks and the tested functions byte-for-byte.
{
    printf 'typedef unsigned uint32_t;\n#define UINT32_C(v) v##U\n'
    printf 'extern void *memcpy(void *,const void *,unsigned);\nextern int puts(const char *);\n'
    sed '/^#include/d' tests/regression/shufflevector_value_convention.c
} > "$tmp/typed.c"
for mode in sse2 i686 no-sse; do
    case $mode in
        sse2) isa_flags=(-msse2) ;;
        i686) isa_flags=(-march=i686) ;;
        no-sse) isa_flags=(-mno-sse) ;;
    esac
    flags=("${base_flags[@]}" "${isa_flags[@]}")
    for driver in "$GCC_ORACLE_BIN" "$CLANG_ORACLE_BIN"; do
        "$driver" "${flags[@]}" -B"$as_dir/" -O2 -c "$fixture/driver.c" -o "$tmp/abi-driver.o"
        "$driver" "${flags[@]}" -B"$as_dir/" -O2 -DSHUFFLE_DRIVER_ONLY -c "$tmp/typed.c" -o "$tmp/typed-driver.o"
        for compiler in "$ccc" "$GCC_ORACLE_BIN" "$CLANG_ORACLE_BIN"; do
            extra=()
            [[ $compiler == "$ccc" ]] || extra=(-B"$as_dir/")
            for opt in O0 O2 O3 Os; do
                "$compiler" "${flags[@]}" "${extra[@]}" -"$opt" -c "$fixture/functions.c" -o "$tmp/functions.o"
                "$linker" -m elf_i386 -static -e _start "$tmp/start.o" "$tmp/abi-driver.o" "$tmp/functions.o" -o "$tmp/abi"
                "$tmp/abi"
                "$compiler" "${flags[@]}" "${extra[@]}" -"$opt" -DSHUFFLE_FUNCTIONS_ONLY -c "$tmp/typed.c" -o "$tmp/functions.o"
                "$linker" -m elf_i386 -static -e _start "$tmp/start.o" "$tmp/typed-driver.o" "$tmp/functions.o" -o "$tmp/typed"
                "$tmp/typed"
                echo "ok: $mode $compiler -$opt / $driver driver: 16384 ABI + 18432 typed shuffle checks"
            done
        done
    done
done
echo 'PASS: i686 GNU vector SSE XMM and pre-SSE/no-SSE stack/sret ABI, indirect calls, stack overflow, variadics and typed shuffles'
