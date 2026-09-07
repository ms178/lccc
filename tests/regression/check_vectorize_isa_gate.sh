#!/usr/bin/env bash
# Emission + semantics gate for the x86 SIMD ISA contract of the middle end.
#
#   default / -march=x86-64          -> AVX2 vectorization (project baseline
#                                       is x86-64-v3; measured benchmark data
#                                       depends on this, so it must not
#                                       silently regress)
#   -mno-avx / -mno-avx2             -> 128-bit SSE2 vectorization: downgraded,
#                                       NOT disabled, and zero ymm
#   -mno-sse -mno-mmx -mno-sse2
#   -mno-avx (the kernel's flag set) -> no SIMD register reference at all
#   fma/fmaf                         -> vfmadd* by default, never under
#                                       -mno-sse / -mno-avx / -mno-fma
#
# Every configuration is also *executed*: a gate that merely stopped
# vectorizing would pass the emission checks while silently degrading or
# breaking the loop, so the semantics are pinned too.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
src=$repo/tests/regression/vectorize_isa_gate.c
fmasrc=$repo/tests/regression/fma_isa_gate.c
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# The exact SIMD-related subset of the kernel's KBUILD_CFLAGS.
KERNEL_ISA=(-mno-sse -mno-mmx -mno-sse2 -mno-3dnow -mno-avx -mno-sse4a)

fail=0
check_eq() { # check_eq <desc> <actual> <expected>
    if [[ "$2" != "$3" ]]; then
        echo "FAIL: $1 -- got '$2', want '$3'" >&2
        fail=1
    fi
}
check_gt0() { # check_gt0 <desc> <actual>
    if [[ "$2" -eq 0 ]]; then
        echo "FAIL: $1 -- expected a non-zero count, got 0" >&2
        fail=1
    fi
}
count_simd() { grep -cE '\b[xyz]mm[0-9]+\b' "$1" || true; }
count_ymm() { grep -cE '\bymm[0-9]+\b' "$1" || true; }
count_vfmadd() { grep -cE '\bvfmadd[0-9]*p?[sd]\b|\bvfmadd' "$1" || true; }

run_cfg() { # run_cfg <label> <flags...> ; asserts emission + execution
    local label=$1; shift
    "$ccc" -O2 "$@" -S "$src" -o "$tmp/$label.s"
    "$ccc" -O2 "$@" "$src" -o "$tmp/$label.bin"
    local out
    out=$("$tmp/$label.bin")
    check_eq "$label semantics" "$out" "fail=0"
}

# ---- 1. default: AVX2 baseline must survive -------------------------------
run_cfg default
check_gt0 "default AVX2 vectorization (ymm)" "$(count_ymm $tmp/default.s)"

run_cfg march-v3 -march=x86-64-v3
check_gt0 "-march=x86-64-v3 AVX2 vectorization (ymm)" "$(count_ymm $tmp/march-v3.s)"

# ---- 2. -mno-avx: downgrade to SSE2, keep vectorizing ---------------------
for flag in -mno-avx -mno-avx2 -mno-sse4.1; do
    lbl=${flag#-m}; lbl=n${lbl#no-}
    run_cfg "$lbl" "$flag"
    check_eq "$flag ymm emission" "$(count_ymm $tmp/$lbl.s)" 0
    check_gt0 "$flag still vectorizes (xmm)" "$(count_simd $tmp/$lbl.s)"
done

# ---- 3. kernel flag set: no SIMD register at all --------------------------
run_cfg kernel-isa "${KERNEL_ISA[@]}"
check_eq "kernel ISA flags: SIMD register refs" "$(count_simd $tmp/kernel-isa.s)" 0

# -O3 runs the same pipeline with unrolling on top.
"$ccc" -O3 "${KERNEL_ISA[@]}" -S "$src" -o "$tmp/kernel-isa-o3.s"
check_eq "kernel ISA flags at -O3: SIMD register refs" \
    "$(count_simd $tmp/kernel-isa-o3.s)" 0

# ---- 4. FMA3 gate ----------------------------------------------------------
"$ccc" -O2 -S "$fmasrc" -o "$tmp/fma-default.s"
check_gt0 "default fmaf folds to vfmadd" "$(count_vfmadd $tmp/fma-default.s)"
for flag in -mno-fma -mno-avx; do
    "$ccc" -O2 "$flag" -S "$fmasrc" -o "$tmp/fma.s"
    check_eq "$flag: vfmadd emission" "$(count_vfmadd $tmp/fma.s)" 0
done
"$ccc" -O2 "${KERNEL_ISA[@]}" -S "$fmasrc" -o "$tmp/fma.s"
check_eq "kernel ISA flags: vfmadd emission" "$(count_vfmadd $tmp/fma.s)" 0

# ---- 5. `-msse` after `-mno-sse` re-enables (kernel CC_FLAGS_FPU) ----------
# arch/x86/Makefile appends -msse to CC_FLAGS_FPU for FPU-using TUs, after the
# global -mno-sse; GCC's last-explicit-ISA-flag-wins must be honoured.
"$ccc" -O2 "${KERNEL_ISA[@]}" -msse -msse2 -S "$src" -o "$tmp/resse.s"
check_gt0 "-mno-sse ... -msse re-enables vectorization" "$(count_simd $tmp/resse.s)"

if [[ $fail -ne 0 ]]; then exit 1; fi
echo "vectorizer/FMA x86 ISA emission gates: PASS"
