#!/usr/bin/env bash
# I32/I64 AVX2/SSE2 reduction accumulators stay in width-aware SIMD homes.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$dir/../.." && pwd)"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-i32-reduction-vecreg.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

flags=(-O3 -march=x86-64-v3)
source_file="$root/tests/benchmark/programs/double_reduction.c"
"$CCC" "${flags[@]}" -S "$source_file" -o "$tmp/avx.s"
CCC_NO_REDUCTION_VECREG=1 \
    "$CCC" "${flags[@]}" -S "$source_file" -o "$tmp/avx-control.s"

LCCC_FORCE_SSE2=1 "$CCC" -O3 -S "$dir/vectorize_sum_i32.c" -o "$tmp/sse.s"
LCCC_FORCE_SSE2=1 CCC_NO_REDUCTION_VECREG=1 \
    "$CCC" -O3 -S "$dir/vectorize_sum_i32.c" -o "$tmp/sse-control.s"

"$CCC" "${flags[@]}" -S "$dir/vectorize_sum_i64.c" -o "$tmp/i64.s"
CCC_NO_REDUCTION_VECREG=1 \
    "$CCC" "${flags[@]}" -S "$dir/vectorize_sum_i64.c" -o "$tmp/i64-control.s"
"$CCC" "${flags[@]}" -S "$dir/vectorize_i64_dot.c" -o "$tmp/i64-dot.s"
CCC_NO_REDUCTION_VECREG=1 \
    "$CCC" "${flags[@]}" -S "$dir/vectorize_i64_dot.c" -o "$tmp/i64-dot-control.s"

python3 - \
    "$tmp/avx.s" "$tmp/avx-control.s" \
    "$tmp/sse.s" "$tmp/sse-control.s" \
    "$tmp/i64.s" "$tmp/i64-control.s" \
    "$tmp/i64-dot.s" "$tmp/i64-dot-control.s" <<'PY'
import re
import sys


def body(path, name):
    text = open(path, encoding="utf-8").read()
    match = re.search(rf"(?ms)^{re.escape(name)}:\n(.*?)^\.size {re.escape(name)},", text)
    if not match:
        raise SystemExit(f"{path}: missing assembly body for {name}")
    return match.group(1)


avx = body(sys.argv[1], "main")
avx_control = body(sys.argv[2], "main")
sse = body(sys.argv[3], "sum_i32")
sse_control = body(sys.argv[4], "sum_i32")
i64 = body(sys.argv[5], "sum_i64")
i64_control = body(sys.argv[6], "sum_i64")
i64_dot = body(sys.argv[7], "dot_i64")
i64_dot_control = body(sys.argv[8], "dot_i64")
stack = r"-?\d+\(%r(?:sp|bp)\)"

# The benchmark contains two two-accumulator loops. Each packed add must update
# a distinct YMM home in place; no loop-carried operand may come from a slot.
direct = re.findall(r"vpaddd %ymm0, %(ymm\d+), %\1", avx)
if len(set(direct)) < 4:
    raise SystemExit(f"AVX2: expected four distinct accumulator homes, got {direct}")
if re.search(rf"vpaddd {stack}", avx):
    raise SystemExit("AVX2: packed accumulator still reads a stack home")
if len(re.findall(rf"vpaddd {stack}", avx_control)) < 4:
    raise SystemExit("AVX2: kill-switch control did not restore four stack accumulators")
new_stores = len(re.findall(rf"vmovdqu %ymm\d+, {stack}", avx))
old_stores = len(re.findall(rf"vmovdqu %ymm\d+, {stack}", avx_control))
if new_stores + 8 > old_stores:
    raise SystemExit(
        f"AVX2: expected at least eight fewer vector stack stores ({new_stores} vs {old_stores})"
    )
if not re.search(r"vpxor %ymm([2-9]|1[0-5]), %ymm\1, %ymm\1", avx):
    raise SystemExit("AVX2: zero producer did not initialize an assigned YMM home directly")

# Forced SSE2 exercises the 128-bit class and register-aware horizontal load.
if not re.search(r"paddd %xmm0, %(xmm(?:[2-9]|1[0-5]))", sse):
    raise SystemExit("SSE2: accumulator is not updated in an XMM home")
if re.search(rf"paddd {stack}", sse):
    raise SystemExit("SSE2: packed accumulator still reads a stack home")
if not re.search(rf"movdqu {stack}, %xmm\d+", sse_control):
    raise SystemExit("SSE2: kill-switch control did not restore the stack accumulator")
if not re.search(r"pxor %xmm([2-9]|1[0-5]), %xmm\1", sse):
    raise SystemExit("SSE2: zero producer did not initialize an assigned XMM home directly")

# I64 sums and software-lane dots use a two-lane SSE2 class.
if not re.search(r"paddq %xmm0, %(xmm(?:[2-9]|1[0-5]))", i64):
    raise SystemExit("I64: accumulator is not updated in an XMM home")
if re.search(rf"paddq {stack}", i64):
    raise SystemExit("I64: packed accumulator still reads a stack home")
if not re.search(rf"movdqu {stack}, %xmm\d+", i64_control):
    raise SystemExit("I64: kill-switch control did not restore the stack accumulator")
if not re.search(r"pxor %xmm([2-9]|1[0-5]), %xmm\1", i64):
    raise SystemExit("I64: zero producer did not initialize an assigned XMM home directly")

if not re.search(r"paddq %xmm0, %(xmm(?:[3-9]|1[0-5]))", i64_dot):
    raise SystemExit("I64 dot: accumulator is not updated in an XMM3+ home")
if re.search(rf"paddq {stack}", i64_dot):
    raise SystemExit("I64 dot: packed accumulator still reads a stack home")
if not re.search(rf"movdqu {stack}, %xmm\d+", i64_dot_control):
    raise SystemExit("I64 dot: kill-switch control did not restore the stack accumulator")
if not re.search(r"pxor %xmm([3-9]|1[0-5]), %xmm\1", i64_dot):
    raise SystemExit("I64 dot: XMM2 scratch quarantine disabled the whole SIMD pool")
PY
