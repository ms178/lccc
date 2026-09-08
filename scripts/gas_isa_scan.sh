#!/usr/bin/env bash
# GAS acceptance + ISA-leak scan over the regression/benchmark corpus.
#
# For every C translation unit in tests/regression and benchmarks, compile
# with lccc under FLAGS to assembly and feed the text to GNU as with
# `-march=ARCH`.  GAS is the authoritative oracle for two independent
# defect classes the integrated assembler cannot report:
#
#   ISA  — an instruction outside the ISA the flags permit (e.g. a VEX
#          `vaddsd` under `-mno-avx`, `pmulld` under `-march=x86-64`, any
#          `%xmm` reference under the kernel's `-mno-sse`).  GAS reports
#          "`mnemonic' is not supported on `ARCH'".
#   SYN  — syntactically wrong AT&T text that the builtin assembler
#          tolerated (a 64-bit register with an `l` suffix, a two-operand
#          `vsqrtsd`, …).  Anything else GAS calls an Error.
#
# ARCH is a GAS `-march=` value; extensions are added with `+`:
#   generic64                        x86-64 baseline (SSE2, no SSE4/AVX)
#   nehalem                          SSE4.2 (x86-64-v2), no AVX
#   haswell                          AVX2+FMA+BMI2 (x86-64-v3)
#   generic64+nosse+nosse2+nommx     the kernel's no-SIMD contract
#   any                              no ISA restriction (syntax scan only)
#
# TUs that opt into an ISA explicitly (intrinsic headers, __builtin_ia32_*,
# __attribute__((target)), inline asm) are skipped unless ARCH is `any`.
#
# Usage:
#   scripts/gas_isa_scan.sh <ARCH> [lccc flags...]
#   LCCC=/path/to/lccc scripts/gas_isa_scan.sh generic64 -O2 -march=x86-64
#   scripts/gas_isa_scan.sh --all         # the standard matrix (CI gate)
#
# Exit status: 0 when no TU fails, 1 otherwise.
set -uo pipefail

repo=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
lccc=${LCCC:-$repo/target/fastbuild/lccc}
as_bin=${AS:-as}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

scan_one() { # scan_one <ARCH> <flags...>
    local arch=$1; shift
    local flags=("$@")
    local n=0 isa=0 syn=0
    local march=()
    [[ $arch != any ]] && march=(-march="$arch")
    for f in "$repo"/tests/regression/*.c "$repo"/benchmarks/*.c; do
        [[ -f $f ]] || continue
        if [[ $arch != any ]]; then
            grep -qE '#include <(imm|emm|xmm|smm|tmm|nmm|pmm|wmm|amm|x86|x86gpr)intrin\.h>|__builtin_ia32|__attribute__\(\(target|__m128|__m256|\basm\b|__asm__' "$f" && continue
        fi
        n=$((n + 1))
        timeout 30 "$lccc" "${flags[@]}" -S "$f" -o "$tmp/t.s" 2>/dev/null || continue
        if ! "$as_bin" --64 "${march[@]}" "$tmp/t.s" -o "$tmp/t.o" 2>"$tmp/err"; then
            if grep -q "is not supported on" "$tmp/err"; then
                isa=$((isa + 1))
                echo "ISA  ${f#"$repo"/}: $(grep -o "\`[^']*' is not" "$tmp/err" | sed "s/' is not//; s/\`//" | sort | uniq -c | sort -rn | head -5 | awk '{printf "%s x%s ", $2, $1}')"
            fi
            if grep -v "is not supported on" "$tmp/err" | grep -q "Error"; then
                syn=$((syn + 1))
                echo "SYN  ${f#"$repo"/}: $(grep -v 'is not supported on' "$tmp/err" | grep Error | head -2 | sed "s|$tmp/t.s:||" | tr '\n' ' ')"
            fi
        fi
    done
    echo "arch=$arch flags='${flags[*]}' total=$n isa_leaks=$isa syntax_defects=$syn"
    [[ $isa -eq 0 && $syn -eq 0 ]]
}

if [[ ${1:-} == --all ]]; then
    rc=0
    scan_one any -O0 || rc=1
    scan_one any -O2 || rc=1
    scan_one any -O3 -march=x86-64-v3 || rc=1
    scan_one generic64 -O2 -march=x86-64 || rc=1
    scan_one generic64 -O3 -march=x86-64 || rc=1
    scan_one nehalem -O2 -mno-avx || rc=1
    scan_one nehalem -O1 -mno-avx || rc=1
    exit $rc
fi

[[ $# -ge 1 ]] || { sed -n '2,32p' "$0"; exit 2; }
scan_one "$@"
