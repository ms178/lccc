#!/usr/bin/env bash
# Structural regression for width-aware register-resident vector reductions.
#
# Two generations of this contract are pinned:
#  * every reduction kernel keeps its packed accumulators register-resident
#    (in-place updates, no vector operand read from a stack slot, no vector
#    store to a stack slot anywhere in the body);
#  * for the dot/sum-squares family, CCC_NO_REDUCTION_VECREG still restores
#    the old spill-backed accumulator web, proving the register homes come
#    from the reduction-vecreg candidate collection on that family.
# The plain-sum family used to have the same kill-switch differential, but
# the general register allocator now homes those accumulators by itself —
# with the switch on, only the epilogue combine round-trips to a slot — so
# that family's differential leg is gone and only the positive (register-
# resident) contract remains. Stack slots are matched as
# `-?\d+\(%r(?:sp|bp)\)`: frames without a frame pointer address them
# through %rsp, and a %rbp-only pattern silently vacated every negative
# assertion when default codegen stopped setting up a frame pointer here.
set -euo pipefail
CCC=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source_file="$dir/../benchmark/patterns/simd_fp_oracle.c"
out=$(mktemp "${TMPDIR:-/tmp}/lccc-reduction-vecreg.XXXXXX.s")
control=$(mktemp "${TMPDIR:-/tmp}/lccc-reduction-stack.XXXXXX.s")
trap 'rm -f "$out" "$control"' EXIT

"$CCC" -O3 -march=x86-64-v3 -ffast-math -ffp-contract=fast \
    -S "$source_file" -o "$out"
CCC_NO_REDUCTION_VECREG=1 \
    "$CCC" -O3 -march=x86-64-v3 -ffast-math -ffp-contract=fast \
    -S "$source_file" -o "$control"

python3 - "$out" "$control" <<'PY'
import re
import sys

new = open(sys.argv[1], encoding="utf-8").read()
old = open(sys.argv[2], encoding="utf-8").read()
STACK = r"-?\d+\(%r(?:sp|bp)\)"


def body(text, name):
    match = re.search(rf"(?ms)^{re.escape(name)}:\n(.*?)^\.size {re.escape(name)},", text)
    if not match:
        raise SystemExit(f"missing assembly body for {name}")
    return match.group(1)


for name, mnemonic in {
    "p15_sum_f32": "vaddps",
    "p16_sum_f64": "vaddpd",
}.items():
    current = body(new, name)
    # In-place accumulator update: the accumulator register appears as
    # BOTH the last source and the destination. The streamed input is
    # memory-folded into the first operand since the VLFOLD work
    # (`vaddps (%rdi,%r9), %ymm2, %ymm2` — one instruction where the
    # pre-fold spelling loaded the stream into %ymm0 first).
    direct = re.search(
        rf"{mnemonic} .*, %(ymm(?:[2-9]|1[0-5])), %\1", current
    )
    if not direct:
        raise SystemExit(f"{name}: vector accumulator is not updated in place")
    if re.search(rf"{mnemonic} .*{STACK}", current):
        raise SystemExit(f"{name}: vector accumulator still loads from its stack slot")

for name, suffix in {
    "p17_dot_f32": "ps",
    "p18_dot_f64": "pd",
    "p23_sum_squares_f32": "ps",
}.items():
    current = body(new, name)
    disabled = body(old, name)
    if not re.search(rf"vfmadd231{suffix} .*%ymm0, %ymm(?:[2-9]|1[0-5])", current):
        raise SystemExit(f"{name}: fused accumulator is not register-resident")
    if re.search(rf"vmov\w* %ymm\d+, {STACK}", current):
        raise SystemExit(f"{name}: fused vector accumulator still spills")
    if not re.search(rf"vmovdqu %ymm0, {STACK}", disabled):
        raise SystemExit(f"{name}: kill-switch control did not restore stack accumulator")

for name in ("p15_sum_f32", "p16_sum_f64"):
    if re.search(rf"vmov\w* %ymm\d+, {STACK}", body(new, name)):
        raise SystemExit(f"{name}: unexpected vector stack store")
PY
