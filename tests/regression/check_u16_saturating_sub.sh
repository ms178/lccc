#!/usr/bin/env bash
# Execute exact boundary, tail, overlap and negative-control cases, and pin the
# SSE2/AVX2 lowering without permitting SIMD when the target forbids it.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$root/target/fastbuild/lccc}
gcc=${GCC:-gcc-16}
src="$root/tests/regression/vec_u16_saturating_sub.c"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fail() { echo "::error::$*" >&2; exit 1; }
function_body() {
    awk -v name="$1" '
        $0 == name ":" { inside = 1; next }
        inside && /^[[:alpha:]_][[:alnum:]_]*:/ { exit }
        inside { print }
    ' "$2"
}

for opt in -O2 -O3; do
    for mode in sse2 avx2 scalar; do
        case "$mode" in
            sse2) flags=(-march=x86-64) ;;
            avx2) flags=(-march=x86-64-v3) ;;
            scalar) flags=(-mno-sse -mno-sse2 -mno-mmx -mno-avx) ;;
        esac
        label="${opt#-}-$mode"
        "$ccc" "$opt" "${flags[@]}" -S "$src" -o "$tmp/$label.s" || fail "$label assembly failed"
        "$ccc" "$opt" "${flags[@]}" "$src" -o "$tmp/$label" || fail "$label LCCC build failed"
        "$gcc" "$opt" "${flags[@]}" "$src" -o "$tmp/$label-gcc" || fail "$label GCC build failed"
        if ! "$tmp/$label" > "$tmp/$label.out"; then
            cat "$tmp/$label.out"
            fail "$label LCCC runtime failed"
        fi
        if ! "$tmp/$label-gcc" > "$tmp/$label-gcc.out"; then
            cat "$tmp/$label-gcc.out"
            fail "$label GCC runtime failed"
        fi
        cmp "$tmp/$label.out" "$tmp/$label-gcc.out" || fail "$label disagrees with GCC"
        function_body sat_ge "$tmp/$label.s" > "$tmp/$label-sat.s"
        function_body sat_max_select "$tmp/$label.s" > "$tmp/$label-sat-max.s"
        function_body signed_compare "$tmp/$label.s" > "$tmp/$label-signed.s"
        function_body nonzero_fallback "$tmp/$label.s" > "$tmp/$label-nonzero.s"
        if [[ "$mode" == sse2 ]]; then
            grep -Eq '\bpsubusw\b' "$tmp/$label-sat.s" || fail "$label lacks SSE2 saturation"
            grep -Eq '\bpsubusw\b' "$tmp/$label-sat-max.s" || fail "$label lacks SSE2 compare/select max saturation"
            if grep -Eq '\b[yz]mm[0-9]+\b|\bvpsubusw\b' "$tmp/$label.s"; then
                fail "$label emitted a post-SSE2 vector instruction"
            fi
        elif [[ "$mode" == avx2 ]]; then
            grep -Eq '\bvpsubusw\b.*%ymm' "$tmp/$label-sat.s" || fail "$label lacks AVX2 saturation"
            grep -Eq '\bvpsubusw\b.*%ymm' "$tmp/$label-sat-max.s" || fail "$label lacks AVX2 compare/select max saturation"
        else
            if grep -Eq '\b[xyz]mm[0-9]+\b|\bv?psubusw\b' "$tmp/$label.s"; then
                fail "$label emitted SIMD with SIMD disabled"
            fi
        fi
        if grep -Eq '\bv?psubusw\b' "$tmp/$label-signed.s" "$tmp/$label-nonzero.s"; then
            fail "$label used unsigned saturation for a negative control"
        fi
    done
done

# The packed operation is an x86-only specialization. Compile a header-free
# fallback witness with the other shipped 64-bit target drivers so an intrinsic
# can never leak into AArch64/RISC-V output. No libc/sysroot is needed here.
cat >"$tmp/cross.c" <<'EOF'
typedef unsigned short u16;
void sat_cross(u16 *restrict d, const u16 *restrict a,
               const u16 *restrict b, int n) {
    for (int i = 0; i < n; ++i)
        d[i] = a[i] > b[i] ? (u16)(a[i] - b[i]) : 0;
}
EOF
for driver in lccc-arm lccc-riscv; do
    cc="$root/target/fastbuild/$driver"
    [[ -x "$cc" ]] || fail "$driver was not produced by the fastbuild target matrix"
    "$cc" -O2 -S "$tmp/cross.c" -o "$tmp/$driver.s" ||
        fail "$driver could not compile the portable saturation fallback"
    if grep -Eq '\bv?psubusw\b|%[xyz]mm[0-9]+' "$tmp/$driver.s"; then
        fail "$driver emitted an x86 packed saturation instruction"
    fi
done
echo 'unsigned word saturating-subtract contracts: PASS'
