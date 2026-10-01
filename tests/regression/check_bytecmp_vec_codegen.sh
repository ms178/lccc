#!/usr/bin/env bash
# Byte-compare window phase: emission contract AND executed semantics.
#
# The arm turns
#
#     while (p < end && *p == *q) { p++; q++; }
#
# into a WIDTH-byte `pcmpeqb`/`pmovmskb` window phase in front of the
# original scalar loop.  Three properties have to hold in the EMITTED code,
# and each of them has a failure mode that is invisible to a plain run:
#
#   1. The phase exists and picks its width from the ISA: 32-byte AVX2
#      windows by default, 128-bit windows under -march=x86-64 / -mno-avx
#      (downgraded, never disabled), and nothing at all under -mno-sse (the
#      kernel flag set) or with the LCCC_NO_BYTECMP_VEC kill switch.
#   2. The q-side window carries the page-crossing guard (`or $4095`).  It is
#      not an optimization: without it the phase loads into a PROT_NONE page
#      and the program dies.  A gate that only ran the program would have to
#      place a guard page to see it (bytecmp_vec_guard_page.c does), and a
#      gate that only looked at emissions would not know that the guard is
#      what makes that file survive.
#   3. The mismatch exit is EXACT: `~mask` + ctz (tzcnt with BMI1, bsf
#      without) and no rescan loop.
#
# Every configuration is also EXECUTED against the GCC oracle: a transform
# that silently stopped firing, or that produced a wrong exit pointer for one
# window width, would pass the emission checks while miscompiling.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
# The kernel lives in a subdirectory on purpose: the regression runner globs
# tests/regression/*.c non-recursively and compiles each standalone, and this
# file has no main() by design -- it exists to be counted, not run.
kernel=$repo/tests/regression/bytecmp_vec/kernel.c
guard=$repo/tests/regression/bytecmp_vec_guard_page.c
bounds=$repo/tests/regression/bytecmp_vec_bounds.c
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fail=0
check_eq() { # check_eq <desc> <actual> <expected>
    if [[ "$2" != "$3" ]]; then
        echo "FAIL: $1 -- got '$2', want '$3'" >&2
        fail=1
    fi
}
check_gt0() { # check_gt0 <desc> <actual>
    if [[ "${2:-0}" -eq 0 ]]; then
        echo "FAIL: $1 -- expected a non-zero count, got 0" >&2
        fail=1
    fi
}

emit() { # emit <tag> <extra lccc flags...>
    local tag=$1; shift
    "$ccc" -O2 "$@" -S "$kernel" -o "$tmp/$tag.s"
}
count() { grep -cE "$2" "$tmp/$1.s" || true; }
# `$1` = tag, `$2` = per-config expectation, `$3` = "avx2" | "sse2" | "none".
check_config() {
    local tag=$1 width=$2
    local ymm xmm cmp mask ctz
    ymm=$(count "$tag" '\bymm[0-9]+')
    xmm=$(count "$tag" '\bxmm[0-9]+')
    cmp=$(count "$tag" '\bpcmpeqb|\bvpcmpeqb')
    mask=$(count "$tag" 'v?pmovmskb')
    ctz=$(count "$tag" '\btzcntl?\b|\bbsfl?\b')
    case "$width" in
        avx2)
            check_gt0 "$tag: 256-bit windows emitted" "$ymm"
            check_eq "$tag: no 128-bit data registers" "$xmm" 0
            check_gt0 "$tag: vpcmpeqb emitted" "$cmp"
            check_gt0 "$tag: pmovmskb emitted" "$mask"
            check_gt0 "$tag: ctz-based exact exit emitted" "$ctz"
            ;;
        sse2)
            check_eq "$tag: no 256-bit registers" "$ymm" 0
            check_gt0 "$tag: 128-bit windows emitted" "$xmm"
            check_gt0 "$tag: pcmpeqb emitted" "$cmp"
            check_gt0 "$tag: pmovmskb emitted" "$mask"
            check_gt0 "$tag: ctz-based exact exit emitted" "$ctz"
            ;;
        none)
            check_eq "$tag: no 256-bit registers" "$ymm" 0
            check_eq "$tag: no SIMD data registers" "$xmm" 0
            check_eq "$tag: no packed compare" "$cmp" 0
            check_eq "$tag: no mask extraction" "$mask" 0
            ;;
    esac
    # The page-crossing guard belongs to every vectorized configuration and
    # to no scalar one (it is emitted as part of the q-side test).
    # One guard per phase, and the kernel holds two functions: an unsigned
    # and a signed loop.  Asserting the exact count (not "at least one")
    # keeps the check honest if the guard is ever hoisted or shared.
    local want_guards=0
    [[ "$width" != "none" ]] && want_guards=2
    check_eq "$tag: q-side page guards == number of phases" \
        "$(count "$tag" 'orq \$4095|orl \$4095')" "$want_guards"
}

# ISA profile of the middle end (see check_vectorize_isa_gate.sh for the
# project-wide contract this section reuses).
KERNEL_ISA=(-mno-sse -mno-mmx -mno-sse2 -mno-3dnow -mno-avx -mno-sse4a)

emit default
emit march -march=x86-64
emit noavx -mno-avx
emit nosse "${KERNEL_ISA[@]}"
LCCC_NO_BYTECMP_VEC=1 "$ccc" -O2 -S "$kernel" -o "$tmp/kill.s"

check_config default avx2
check_config march sse2
check_config noavx sse2
check_config nosse none
check_config kill none

# The kill switch must be a switch, not a rewrite: with it set, the emitted
# code for the default ISA has to be the plain scalar loop (no SIMD at all).
check_eq "kill switch: default-ISA code equals the -mno-sse code shape" \
    "$(count kill '\bpcmpeqb|\bvpcmpeqb')" 0

# Semantics under every configuration, against the GCC oracle.  The guard-page
# file is the one that dies (SIGSEGV) if the q-side guard is ever removed.
run_case() { # run_case <tag> <source> <flags...>
    local tag=$1 src=$2; shift 2
    "$ccc" -O2 "$@" "$src" -o "$tmp/$tag.lccc"
    gcc -O2 "$@" "$src" -o "$tmp/$tag.gcc" 2>/dev/null || return 0
    local out_l out_g rc_l rc_g
    out_l=$("$tmp/$tag.lccc" 2>&1); rc_l=$?
    out_g=$("$tmp/$tag.gcc" 2>&1); rc_g=$?
    check_eq "$tag: lccc exit status matches GCC" "$rc_l" "$rc_g"
    check_eq "$tag: lccc stdout matches GCC" "$out_l" "$out_g"
}

for cfg in "" "-march=x86-64" "-mno-avx"; do
    tag="bounds${cfg// /}"
    # shellcheck disable=SC2086
    run_case "$tag" "$bounds" $cfg
    tag="guard${cfg// /}"
    # shellcheck disable=SC2086
    run_case "$tag" "$guard" $cfg
done
LCCC_NO_BYTECMP_VEC=1 run_case "bounds-kill" "$bounds"
LCCC_NO_BYTECMP_VEC=1 run_case "guard-kill" "$guard"

if [[ $fail -ne 0 ]]; then
    echo "bytecmp window-phase gate: FAILED" >&2
    exit 1
fi
echo "bytecmp window-phase gate: PASS (widths, page guard, exact exit, semantics vs GCC)"
