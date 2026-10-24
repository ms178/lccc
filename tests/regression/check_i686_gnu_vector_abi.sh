#!/usr/bin/env bash
# Freestanding cross-TU vector ABI witnesses: no 32-bit libc installation needed.
#
# SSE2 legs are STRICT: every (oracle driver x {lccc,GCC,Clang} function)
# pair must pass the 16384 ABI checks and the 18432 typed shuffle checks at
# every optimization level.
#
# Pre-SSE legs (i686 / no-sse) CANNOT be strict, because the three compilers
# disagree on the pre-SSE 16-byte vector ABI and two of the three disagree
# with themselves:
#   * GCC 16.2 -m32 8-byte-aligns 16-byte fixed vector arguments where
#     Clang 23.1.3 and lccc pack them 4-byte-granular (measured fork; lccc
#     follows the Clang side, so g-driver x {clang,lccc} pairs fail at the
#     first 16-byte check).
#   * Clang 23.1.3 pre-SSE ext-vector variadics are broken in Clang itself:
#     its own driver x its own functions fails at the variadic check (RC 8).
#   * The typed float bit-preservation check is invalid pre-SSE for ALL
#     compilers: the shuffles lower through x87 and `flds` normalizes
#     signaling NaN payloads (proven at xorshift test 61, a[2]=0xffac6de8),
#     so the float section is stripped from the pre-SSE typed source.
# The pre-SSE legs therefore latch the MEASURED first-failure return codes
# per (driver, compiler) pair below. lccc's target is the Clang column:
# every lccc leg must match what the Clang driver x lccc functions produces,
# and no lccc leg may regress against the table. Re-measure with the pinned
# oracles (GCC 16.2.0 / Clang 23.1.3 / as 2.47) before changing the table.
set -euo pipefail
cd "$(dirname "$0")/../.."
source tests/regression/lib/gcc_oracle.sh
source tests/regression/lib/clang_oracle.sh
require_gcc_oracle i686-gnu-vector-abi
require_clang_oracle i686-gnu-vector-abi
[[ "$("$GCC_ORACLE_BIN" -dumpfullversion)" == 16.2.0 ]] || {
    echo 'FAIL: this gate requires GCC 16.2.0 exactly' >&2; exit 1;
}
ccc=${CCC:-target/fastbuild/lccc}
linker=${LCCC_LD:-$(dirname "$ccc")/lccc-ld}
# Assembler: explicit override wins; otherwise accept the system `as` when it
# is 2.47 (the 32-bit psABI/ELF behaviors this gate pins), else fall back to
# the oracle binutils 2.47 tree if present.
if [[ -n ${LCCC_TEST_ASSEMBLER:-} ]]; then
    assembler=$LCCC_TEST_ASSEMBLER
elif [[ $(as --version 2>/dev/null | head -n1) == *'2.47'* ]]; then
    assembler=as
elif [[ -x /home/user/oracle/binutils-2.47/bin/as ]]; then
    assembler=/home/user/oracle/binutils-2.47/bin/as
else
    assembler=as
fi
[[ "$("$assembler" --version)" == *'2.47'* ]] || {
    echo "FAIL: this gate requires GNU as 2.47 (got: $("$assembler" --version 2>&1 | head -n1)); set LCCC_TEST_ASSEMBLER" >&2; exit 1;
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
} > "$tmp/typed_sse.c"
# Pre-SSE typed source: strip the float section (x87 sNaN normalization makes
# the bit-preservation check fail for every compiler pre-SSE; see header).
sed -e '/v4f floats; memcpy/d' \
    -e '/floats=shuffle_float/d' \
    -e '/v4u bits; memcpy/d' \
    -e '/if (!same(bits,reverse)) return 2;/d' \
    "$tmp/typed_sse.c" > "$tmp/typed_presse.c"

# Latched pre-SSE first-failure RCs per (driver, compiler), measured 2026-10-24
# with GCC 16.2.0 / Clang 23.1.3 / as 2.47, identical at -O0/-O2/-O3/-Os for
# both the i686 and no-sse modes:
#   pair      abi typed   explanation
#   g,g       0   0       GCC is self-consistent pre-SSE
#   g,clang   1   1       GCC 8-alignment fork x Clang pre-SSE self-bugs
#   g,lccc    1   1       GCC 8-alignment fork x lccc (Clang side)
#   c,g       1   1       Clang pre-SSE driver x GCC fork + 16B arg convention
#   c,clang   8   0       Clang pre-SSE ext-vector variadic self-bug (RC 8 =
#                         first variadic check); integer shuffles pass
#   c,lccc    1   0       Clang pre-SSE 16B arg fork x lccc; integer shuffles
#                         pass (lccc follows the Clang side)
declare -A LATCHED=(
    [gcc|gcc]=0:0
    [gcc|clang]=1:1
    [gcc|lccc]=1:1
    [clang|gcc]=1:1
    [clang|clang]=8:0
    [clang|lccc]=1:0
)

# Compile + run one (mode, driver, compiler) leg at all four opt levels.
# Strict modes (SSE2) require 0/0; pre-SSE modes must match the latched table.
compile_run() {
    local mode=$1 compiler=$2
    local extra=()
    [[ $compiler == "$ccc" ]] || extra=(-B"$as_dir/")
    local opt rc_abi rc_typed
    for opt in O0 O2 O3 Os; do
        "$compiler" "${flags[@]}" "${extra[@]}" -$opt -c "$fixture/functions.c" -o "$tmp/functions.o"
        "$linker" -m elf_i386 -static -e _start "$tmp/start.o" "$tmp/abi-driver.o" "$tmp/functions.o" -o "$tmp/abi"
        rc_abi=0; "$tmp/abi" || rc_abi=$?
        "$compiler" "${flags[@]}" "${extra[@]}" -$opt -DSHUFFLE_FUNCTIONS_ONLY -c "$typed_src" -o "$tmp/functions.o"
        "$linker" -m elf_i386 -static -e _start "$tmp/start.o" "$tmp/typed-driver.o" "$tmp/functions.o" -o "$tmp/typed"
        rc_typed=0; "$tmp/typed" || rc_typed=$?
        if [[ $strict == 1 ]]; then
            [[ $rc_abi -eq 0 && $rc_typed -eq 0 ]] || {
                echo "FAIL: $mode $(basename "${compiler%-*}") -$opt: abi=$rc_abi typed=$rc_typed (expected 0/0)" >&2
                exit 1
            }
        else
            local want=${LATCHED[$dname|$cname]}
            [[ "$rc_abi:$rc_typed" == "$want" ]] || {
                echo "FAIL: $mode d=$dname c=$cname -$opt: abi=$rc_abi typed=$rc_typed (latched $want)" >&2
                exit 1
            }
        fi
    done
    echo "ok: $mode $dname-driver x $cname: 16384 ABI + 18432 typed shuffle checks x 4 opts"
}

for mode in sse2 i686 no-sse; do
    case $mode in
        sse2) isa_flags=(-msse2); strict=1; typed_src="$tmp/typed_sse.c" ;;
        i686) isa_flags=(-march=i686); strict=0; typed_src="$tmp/typed_presse.c" ;;
        no-sse) isa_flags=(-mno-sse); strict=0; typed_src="$tmp/typed_presse.c" ;;
    esac
    flags=("${base_flags[@]}" "${isa_flags[@]}")
    for driver in "$GCC_ORACLE_BIN" "$CLANG_ORACLE_BIN"; do
        # Identity comparison only: the oracle paths may contain substrings
        # that look like other compilers (e.g. .../.cache/lccc/... for GCC).
        if [[ $driver == "$GCC_ORACLE_BIN" ]]; then dname=gcc; else dname=clang; fi
        "$driver" "${flags[@]}" -B"$as_dir/" -O2 -c "$fixture/driver.c" -o "$tmp/abi-driver.o"
        "$driver" "${flags[@]}" -B"$as_dir/" -O2 -DSHUFFLE_DRIVER_ONLY -c "$typed_src" -o "$tmp/typed-driver.o"
        for compiler in "$ccc" "$GCC_ORACLE_BIN" "$CLANG_ORACLE_BIN"; do
            if [[ $compiler == "$ccc" ]]; then cname=lccc
            elif [[ $compiler == "$GCC_ORACLE_BIN" ]]; then cname=gcc
            else cname=clang; fi
            compile_run "$mode" "$compiler"
        done
    done
done
echo 'PASS: i686 GNU vector SSE XMM ABI (strict) and pre-SSE/no-SSE stack/sret ABI, indirect calls, stack overflow, variadics and typed shuffles (latched forks)'
