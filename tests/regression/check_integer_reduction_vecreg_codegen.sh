#!/usr/bin/env bash
# I32/I64 AVX2/SSE2 reduction accumulators stay in width-aware SIMD homes.
#
# Pattern notes (why the regexes look the way they do):
#  * the streamed operand is memory-folded since the VLFOLD work
#    (`vpaddd (%rdx,%r8), %ymm6, %ymm6`), so an in-place update is matched
#    as `<op> <anything>, %REG, %REG` with the same register twice, not by
#    pinning the streamed operand to %ymm0;
#  * the I64 legs lower through VEX (`vpaddq`), not the legacy two-operand
#    `paddq`, at x86-64-v3;
#  * only the AVX2 benchmark leg still has a CCC_NO_REDUCTION_VECREG
#    differential (12 vector stack stores/loads return). The SSE2 and I64
#    legs are now homed identically by the general register allocator with
#    the switch on, so their kill-switch control legs were removed rather
#    than left asserting a differential that no longer exists; the positive
#    register-resident contract is unchanged.
set -euo pipefail
CCC=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
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
"$CCC" "${flags[@]}" -S "$dir/vectorize_sum_i64.c" -o "$tmp/i64.s"
"$CCC" "${flags[@]}" -S "$dir/vectorize_i64_dot.c" -o "$tmp/i64-dot.s"

python3 - \
    "$tmp/avx.s" "$tmp/avx-control.s" \
    "$tmp/sse.s" "$tmp/i64.s" "$tmp/i64-dot.s" <<'PY'
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
i64 = body(sys.argv[4], "sum_i64")
i64_dot = body(sys.argv[5], "dot_i64")
stack = r"-?\d+\(%r(?:sp|bp)\)"

# The benchmark contains two two-accumulator loops. Each packed add must
# update its YMM home in place (the streamed operand is memory-folded or in
# a scratch register); sequential loops may share homes, so pin two
# simultaneous homes across at least four in-place update sites, and no
# loop-carried operand may come from a slot.
direct = re.findall(r"vpaddd .*, %(ymm(?:[2-9]|1[0-5])), %\1", avx)
if len(direct) < 4 or len(set(direct)) < 2:
    raise SystemExit(f"AVX2: expected in-place accumulator updates in >=2 homes, got {direct}")
if re.search(rf"vpaddd {stack}", avx):
    raise SystemExit("AVX2: packed accumulator still reads a stack home")
# The switch differential on this leg is in spill traffic: the control
# round-trips every accumulator through slots (loads AND stores).
new_round_trips = len(re.findall(rf"vmovdqu (%ymm\d+, {stack}|{stack}, %ymm\d+)", avx))
old_round_trips = len(re.findall(rf"vmovdqu (%ymm\d+, {stack}|{stack}, %ymm\d+)", avx_control))
if new_round_trips + 8 > old_round_trips:
    raise SystemExit(
        f"AVX2: expected at least eight fewer vector stack round-trips "
        f"({new_round_trips} vs {old_round_trips})"
    )
if not re.search(r"vpxor %ymm([2-9]|1[0-5]), %ymm\1, %ymm\1", avx):
    raise SystemExit("AVX2: zero producer did not initialize an assigned YMM home directly")

# Forced SSE2 exercises the 128-bit class and register-aware horizontal load.
if not re.search(r"paddd %xmm\d+, %(xmm(?:[2-9]|1[0-5]))", sse):
    raise SystemExit("SSE2: accumulator is not updated in an XMM home")
if re.search(rf"paddd {stack}", sse):
    raise SystemExit("SSE2: packed accumulator still reads a stack home")
if not re.search(r"pxor %xmm([2-9]|1[0-5]), %xmm\1", sse):
    raise SystemExit("SSE2: zero producer did not initialize an assigned XMM home directly")

# I64 sums and software-lane dots use a two-lane 128-bit class (VEX forms
# at x86-64-v3, hence the optional `v` and the in-place spelling).
if not re.search(r"vpaddq .*, %(xmm(?:[2-9]|1[0-5])), %\1", i64):
    raise SystemExit("I64: accumulator is not updated in an XMM home")
if re.search(rf"v?paddq {stack}", i64):
    raise SystemExit("I64: packed accumulator still reads a stack home")
if not re.search(r"pxor %xmm([2-9]|1[0-5]), %xmm\1", i64):
    raise SystemExit("I64: zero producer did not initialize an assigned XMM home directly")

if not re.search(r"vpaddq .*, %(xmm(?:[3-9]|1[0-5])), %\1", i64_dot):
    raise SystemExit("I64 dot: accumulator is not updated in an XMM3+ home")
if re.search(rf"v?paddq {stack}", i64_dot):
    raise SystemExit("I64 dot: packed accumulator still reads a stack home")
if not re.search(r"pxor %xmm([3-9]|1[0-5]), %xmm\1", i64_dot):
    raise SystemExit("I64 dot: XMM2 scratch quarantine disabled the whole SIMD pool")
PY
