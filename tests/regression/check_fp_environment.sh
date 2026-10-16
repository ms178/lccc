#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${CCC:-target/fastbuild/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
src=tests/regression/strict_recip_traps.c
# The -m32 legs need a complete 32-bit glibc (bits/*.h + crt objects); probe
# once and shrink the width matrix instead of failing on hosts without it,
# matching the sibling gates' "SKIP -m32" policy.
source "$(dirname "$0")/i386_exec.sh"
widths=(native)
if i386_link_ok "${GCC:-gcc}"; then
    widths+=(i686)
else
    echo "SKIP -m32: no 32-bit toolchain" >&2
fi
for opt in -O0 -O1 -O2 -O3 -Os -Oz; do
    for mode in pragma flag both; do
        flags=()
        case $mode in
            flag) flags=(-DNO_FENV_PRAGMA -ftrapping-math);;
            both) flags=(-ftrapping-math);;
        esac
        "$ccc" "$opt" -march=x86-64-v3 -ffp-contract=off "${flags[@]}" "$src" -lm -o "$td/trap"
        "$td/trap"
        echo "FP first-trap order: $opt $mode PASS"
    done
done
# No observable FP environment requested: the exact same loop must remain
# eligible. This is a structural check, not execution with contradictory
# trapping assumptions (-fno-trapping-math plus feenableexcept).
"$ccc" -O2 -march=x86-64-v3 -DNO_FENV_PRAGMA -ftrapping-math -fno-trapping-math -ffp-contract=off -S "$src" -o "$td/free.s"
grep -q 'vdivpd' "$td/free.s"
"$ccc" -O2 -march=x86-64-v3 -DNO_FENV_PRAGMA -fno-trapping-math -ftrapping-math -ffp-contract=off -S "$src" -o "$td/trapping.s"
if grep -q 'vdivpd' "$td/trapping.s"; then
    echo 'FAIL: packed reciprocal escaped observable-FP environment fence' >&2; exit 1
fi

# Rounding is independent of traps. Also cover SCCP's direct evaluator,
# runtime lowering casts and constant-return/purity inference on both ABIs.
for opt in -O0 -O1 -O2 -O3 -Os -Oz; do
    for width in "${widths[@]}"; do
        abi=()
        if [[ $width == i686 ]]; then abi=(-m32 -msse2); fi
        for mode in pragma flag both; do
            flags=()
            case $mode in
                flag) flags=(-DNO_FENV_PRAGMA -frounding-math);;
                both) flags=(-frounding-math);;
            esac
            "$ccc" "$opt" "${abi[@]}" "${flags[@]}" \
                tests/regression/fenv_constant_rounding.c -lm -o "$td/round"
            "$td/round"
            echo "FP dynamic rounding and discarded exception: $opt $width $mode PASS"
        done
    done
done

# Exact F64 evaluation and usable system <float.h>, including x87 caching.
for opt in -O0 -O1 -O2 -O3 -Os -Oz; do
    for width in "${widths[@]}"; do
        abi=()
        if [[ $width == i686 ]]; then abi=(-m32 -msse2); fi
        "$ccc" "$opt" "${abi[@]}" -ffp-contract=off \
            tests/regression/i686_x87_f64_barrier.c -o "$td/precision"
        "$td/precision"
        echo "Declared F64 precision and float.h: $opt $width PASS"
    done
done

# Runtime rounding and exceptions must not be folded by SCCP even when both
# arithmetic operands are literal constants. Keep the pragma and flag routes.
for opt in -O0 -O1 -O2 -O3 -Os -Oz; do
    for mode in pragma flag; do
        flags=()
        [[ $mode != flag ]] || flags=(-DNO_FENV_PRAGMA -frounding-math)
        "$ccc" "$opt" -ffp-contract=off "${flags[@]}" tests/regression/fenv_constant_effects.c -lm -o "$td/constants"
        "$td/constants"
        echo "FP constant-operand environment effects: $opt $mode PASS"
    done
done
