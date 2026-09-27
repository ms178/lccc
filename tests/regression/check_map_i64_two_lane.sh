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
    python3 - "$arch" "$tmp/vector.s" "$tmp/scalar.s" <<'PY'
import re, sys
arch, vector, scalar = sys.argv[1:]
def body(path):
    text = open(path, encoding='utf8').read()
    match = re.search(r'(?ms)^sub64:\n(.*?)^\.size sub64,', text)
    if match is None:
        raise SystemExit('missing sub64 assembly')
    return match.group(1)
v, s = body(vector), body(scalar)
needle = 'vpsubq' if arch == 'x86-64-v3' else 'psubq'
packed = re.findall(r'\b' + needle + r'\b', v)
if len(packed) != 4:
    raise SystemExit(f'{arch}: expected four independent two-lane {needle} operations, got {len(packed)}')
vector_body = v.split('.LBB4:', 1)[0]
if 'addq $64' not in vector_body:
    raise SystemExit(f'{arch}: I64 map did not advance by the four-copy 64-byte chunk')
if re.search(r'\bleaq\s+(?:16|32|48)\(', vector_body):
    raise SystemExit(f'{arch}: unrolled I64 map materialized a displacement with LEA')
if '%ymm' in v:
    raise SystemExit(f'{arch}: 64-bit map advanced by four lanes but only used XMM')
if re.search(r'\b(?:vpsubq|psubq)\b', s):
    raise SystemExit('scalar kill switch did not suppress packed subtraction')
PY
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
