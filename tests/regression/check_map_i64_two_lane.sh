#!/usr/bin/env bash
# Runtime/differential and codegen gate for the two-lane x86-64 I64 map path.
# The codegen assertions also pin the adaptive four-copy unroll: four 128-bit
# operations, one 64-byte loop step, and displacement operands rather than
# per-copy LEAs.  This is deliberately a shape contract, not a fake 256-bit
# lane-width contract.
set -euo pipefail
CCC=${CCC:-target/fastbuild/lccc}
source_file=tests/regression/map_i64_two_lane.c
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-map64.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

for arch in x86-64-v3 x86-64; do
    flags=(-O2 "-march=$arch")
    "$CCC" "${flags[@]}" "$source_file" -o "$tmp/lccc"
    gcc "${flags[@]}" "$source_file" -o "$tmp/gcc"
    CCC_NO_MAP_VEC=1 "$CCC" "${flags[@]}" "$source_file" -o "$tmp/scalar"
    "$tmp/lccc" > "$tmp/lccc.txt"
    "$tmp/gcc" > "$tmp/gcc.txt"
    "$tmp/scalar" > "$tmp/scalar.txt"
    cmp "$tmp/lccc.txt" "$tmp/gcc.txt"
    cmp "$tmp/lccc.txt" "$tmp/scalar.txt"
    "$CCC" "${flags[@]}" -S "$source_file" -o "$tmp/vector.s"
    CCC_NO_MAP_VEC=1 "$CCC" "${flags[@]}" -S "$source_file" -o "$tmp/scalar.s"
    CCC_NO_MAP_I64_UNROLL=1 "$CCC" "${flags[@]}" -S "$source_file" -o "$tmp/rolled.s"
    CCC_NO_MAP_I64_UNROLL=1 "$CCC" "${flags[@]}" "$source_file" -o "$tmp/rolled"
    "$tmp/rolled" > "$tmp/rolled.txt"
    cmp "$tmp/lccc.txt" "$tmp/rolled.txt"
    small=tests/regression/map_i64_small_trip.c
    "$CCC" "${flags[@]}" -S "$small" -o "$tmp/small.s"
    for mode in vector rolled scalar; do
        case $mode in
            vector) env_args=();;
            rolled) env_args=(CCC_NO_MAP_I64_UNROLL=1);;
            scalar) env_args=(CCC_NO_MAP_VEC=1);;
        esac
        env "${env_args[@]}" "$CCC" "${flags[@]}" "$small" -o "$tmp/small"
        "$tmp/small" > "$tmp/small-$mode.txt"
    done
    gcc "${flags[@]}" "$small" -o "$tmp/small-gcc"
    "$tmp/small-gcc" > "$tmp/small-gcc.txt"
    for mode in vector rolled scalar; do cmp "$tmp/small-gcc.txt" "$tmp/small-$mode.txt"; done
    python3 tests/regression/check_map_i64_shapes.py "$tmp/vector.s" "$tmp/rolled.s" "$tmp/scalar.s" "$tmp/small.s"

done

# i686 has no register-based Vec*I64x2 lowering. Even with SSE2 enabled it
# must emit scalar code and successfully assemble; do not accidentally gate
# on the compiler host architecture instead of the target translation unit.
"$CCC" -O2 -m32 -msse2 -S "$source_file" -o "$tmp/i686.s"
"$CCC" -O2 -m32 -msse2 -c "$source_file" -o "$tmp/i686.o"
if grep -Eq '\b(vpsubq|psubq)\b' "$tmp/i686.s"; then
    echo 'i686 unexpectedly emitted a packed I64 map operation' >&2
    exit 1
fi
echo 'map-i64-two-lane: x86-64 AVX2/SSE2 runtime+GCC+scalar, i686 scalar assembly PASS'
