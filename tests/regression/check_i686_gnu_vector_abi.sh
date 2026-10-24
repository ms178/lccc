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
cat > "$tmp/feature-macros.c" <<'EOF'
#if EXPECT_SSE
#ifndef __SSE__
#error "SSE macro missing"
#endif
#else
#ifdef __SSE__
#error "SSE macro escaped its target ceiling"
#endif
#endif
#if EXPECT_SSE2
#ifndef __SSE2__
#error "SSE2 macro missing"
#endif
#else
#ifdef __SSE2__
#error "SSE2 macro escaped its target ceiling"
#endif
#endif
#if EXPECT_SSE3
#ifndef __SSE3__
#error "SSE3 macro missing"
#endif
#else
#ifdef __SSE3__
#error "SSE3 macro escaped its target ceiling"
#endif
#endif
int feature_macro_contract(void) { return 0; }
EOF
cat > "$tmp/isa-ceiling.c" <<'EOF'
typedef unsigned V4SI __attribute__((vector_size(16)));
V4SI reverse(V4SI value) {
    return __builtin_shufflevector(value, value, 3, 2, 1, 0);
}
EOF
configurations=0
for mode in default sse2 sse-only sse2-denied arch-denied sse-restored no-sse; do
    case $mode in
        default)
            isa_flags=()
            expect_sse=1 expect_sse2=1 expect_sse3=0 expect_pshufd=1 expect_xmm=1
            ;;
        sse2)
            isa_flags=(-msse2)
            expect_sse=1 expect_sse2=1 expect_sse3=0 expect_pshufd=1 expect_xmm=1
            ;;
        sse-only)
            isa_flags=(-msse -mno-sse2)
            expect_sse=1 expect_sse2=0 expect_sse3=0 expect_pshufd=0 expect_xmm=1
            ;;
        sse2-denied)
            isa_flags=(-mno-sse2)
            expect_sse=1 expect_sse2=0 expect_sse3=0 expect_pshufd=0 expect_xmm=1
            ;;
        arch-denied)
            isa_flags=(-mno-sse2 -march=nehalem)
            expect_sse=1 expect_sse2=0 expect_sse3=0 expect_pshufd=0 expect_xmm=1
            ;;
        sse-restored)
            isa_flags=(-mno-sse -msse)
            expect_sse=1 expect_sse2=1 expect_sse3=0 expect_pshufd=1 expect_xmm=1
            ;;
        no-sse)
            isa_flags=(-mno-sse)
            expect_sse=0 expect_sse2=0 expect_sse3=0 expect_pshufd=0 expect_xmm=0
            ;;
    esac
    flags=("${base_flags[@]}" "${isa_flags[@]}")
    for compiler in "$ccc" "$GCC_ORACLE_BIN" "$CLANG_ORACLE_BIN"; do
        extra=()
        [[ $compiler == "$ccc" ]] || extra=(-B"$as_dir/")
        "$compiler" "${flags[@]}" "${extra[@]}" \
            -DEXPECT_SSE="$expect_sse" -DEXPECT_SSE2="$expect_sse2" \
            -DEXPECT_SSE3="$expect_sse3" \
            -c "$tmp/feature-macros.c" -o "$tmp/feature-macros.o"
    done

    # A host executing an SSE2-capable CPU cannot detect an ISA-ceiling
    # escape. Compile a one-source shuffle and require the PSHUFD fast path
    # exactly when SSE2 is effective; the portable gather must remain correct
    # in the runtime matrix below. XMM presence independently proves which
    # argument/return convention this function used.
    assembly="$tmp/isa-$mode.s"
    "$ccc" "${flags[@]}" -O2 -S "$tmp/isa-ceiling.c" -o "$assembly"
    pshufd_count=$(awk '/[[:space:]]v?pshufd[[:space:]]/{n++} END{print n+0}' "$assembly")
    xmm_count=$(awk '/%xmm/{n++} END{print n+0}' "$assembly")
    if (( expect_pshufd ? pshufd_count == 0 : pshufd_count != 0 )); then
        echo "FAIL: $mode PSHUFD ceiling (count=$pshufd_count expected=$expect_pshufd)" >&2
        cat "$assembly" >&2
        exit 1
    fi
    if (( expect_xmm ? xmm_count == 0 : xmm_count != 0 )); then
        echo "FAIL: $mode XMM carrier convention (count=$xmm_count expected=$expect_xmm)" >&2
        cat "$assembly" >&2
        exit 1
    fi
    echo "ok: $mode macros + ISA ceiling (pshufd=$pshufd_count xmm=$xmm_count)"

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
                configurations=$((configurations + 1))
                echo "ok: $mode $compiler -$opt / $driver driver: 16384 ABI + 18432 typed shuffle checks"
            done
        done
    done
done
[[ $configurations == 168 ]] || {
    echo "FAIL: executed $configurations configurations, expected 168" >&2
    exit 1
}
echo 'PASS: 168 i686 GNU vector configurations; default/SSE-only/SSE2/no-SSE ABI, flag-order macro ceilings, PSHUFD ceilings, indirect calls, stack overflow, variadics and typed shuffles'
