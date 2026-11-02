#!/usr/bin/env bash
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
GCC=${GCC:-${GCC_BIN:-gcc-16}}
root=$(cd "$(dirname "$0")/../.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
src="$root/tests/regression/fixtures/strict-packed"
flags=(-O2 -fwrapv -ffp-contract=off)
renames=(-Dpacked_poly=actual_packed_poly -Dpacked_bits=actual_packed_bits -Dscalar_shift=actual_scalar_shift)
for arch in x86-64-v3 x86-64; do
    "$GCC" "${flags[@]}" "-march=$arch" -c "$src/kernels.c" -o "$work/ref.o"
    for homes in normal stack all-stack no-promotion scalar-denominators; do
        env_args=()
        case $homes in
            stack) env_args=(CCC_NO_XMM_REGALLOC=1);;
            all-stack) env_args=(CCC_NO_REGALLOC=1);;
            no-promotion) env_args=(CCC_NO_VEX_PROMOTE=1);;
            scalar-denominators) env_args=(CCC_DISABLE_PASSES=strict_recip_pack);;
        esac
        env "${env_args[@]}" CCC_VERIFY_IR=abort CCC_VALIDATE_SSA=1 "$CCC" \
            "${flags[@]}" "-march=$arch" "${renames[@]}" -c "$src/kernels.c" -o "$work/got.o"
        "$GCC" "${flags[@]}" "-march=$arch" "$src/driver.c" "$work/ref.o" "$work/got.o" -lm -o "$work/run"
        "$work/run"
    done
done
# The retained 128-bit integer DAG uses the normal VEX-promotion pass.
# Test no legacy transitions in the production pipeline; the disabled-pass
# diagnostic above still runs all bitwise/exception/guard-page witnesses.
"$CCC" "${flags[@]}" -march=x86-64-v3 -S "$src/kernels.c" -o "$work/packed.s"
"$CCC" "${flags[@]}" -march=x86-64 -S "$src/kernels.c" -o "$work/scalar.s"
"$CCC" "${flags[@]}" -march=x86-64-v2 -mavx -msse4.1 -S "$src/kernels.c" -o "$work/avx.s"
"$CCC" "${flags[@]}" -mavx2 -mno-sse4.1 -S "$src/kernels.c" -o "$work/no-sse41.s"
python3 - "$work/packed.s" "$work/scalar.s" "$work/avx.s" "$work/no-sse41.s" <<'PY'
import re, sys
packed, scalar, avx, no_sse41 = (open(p).read() for p in sys.argv[1:])
def body(text, name):
    m = re.search(rf'(?ms)^{name}:\n(.*?)^\.size {name},', text)
    assert m, name
    return m[1]
for name, operation in [('packed_poly', 'vpmulld'), ('packed_bits', 'vpxor')]:
    code = body(packed, name)
    assert operation in code and 'vcvtdq2pd' in code and 'vdivpd' in code, name
    assert len(re.findall(r'\bvaddsd\b', code)) >= 4, name
    assert not re.search(r'\bvaddpd\b', code), name
    assert not re.search(r'\b(?:movdqa|movdqu|paddd|pmulld|pinsrd)\b', code), name
    for fallback in [scalar, avx, no_sse41]:
        assert 'vdivpd' not in body(fallback, name), name
    sse = body(scalar, name)
    assert len(re.findall(r'\bcvtdq2pd\b', sse)) >= 2, name
    assert len(re.findall(r'\bdivpd\b', sse)) >= 2, name
    assert len(re.findall(r'\bmulpd\b', sse)) >= 2, name
    assert len(re.findall(r'\baddsd\b', sse)) >= 4, name
    assert not re.search(r'\b(?:addpd|pinsrd|pmulld)\b|%ymm', sse), name
    if name == 'packed_poly':
        assert 'pmuludq' in sse, 'SSE2 multiplication fallback disappeared'
assert 'vpinsrd' in body(packed, 'scalar_shift'), 'variable-shift scalar-DAG fallback lost'
PY

# 32-bit ABI/runtime coverage: SSE arithmetic avoids GCC x87 excess precision.
"$GCC" "${flags[@]}" -m32 -msse2 -mfpmath=sse -c "$src/kernels.c" -o "$work/ref32.o"
CCC_VERIFY_IR=abort CCC_VALIDATE_SSA=1 "$CCC" "${flags[@]}" -m32 -msse2 "${renames[@]}" \
    -c "$src/kernels.c" -o "$work/got32.o"
"$GCC" "${flags[@]}" -m32 -msse2 -mfpmath=sse "$src/driver.c" "$work/ref32.o" "$work/got32.o" -lm -o "$work/run32"
"$work/run32"
