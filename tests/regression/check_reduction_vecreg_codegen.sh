#!/usr/bin/env bash
# Structural regression for width-aware register-resident vector reductions.
set -euo pipefail
CCC=${CCC:-./target/release/lccc}
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


def body(text, name):
    match = re.search(rf"(?ms)^{re.escape(name)}:\n(.*?)^\.size {re.escape(name)},", text)
    if not match:
        raise SystemExit(f"missing assembly body for {name}")
    return match.group(1)


cases = {
    "p15_sum_f32": "vaddps",
    "p16_sum_f64": "vaddpd",
    "p17_dot_f32": "vaddps",
    "p18_dot_f64": "vaddpd",
    "p23_sum_squares_f32": "vaddps",
}
for name, mnemonic in cases.items():
    current = body(new, name)
    disabled = body(old, name)
    direct = re.search(
        rf"{mnemonic} %ymm0, %(ymm(?:[2-9]|1[0-5])), %\1", current
    )
    if not direct:
        raise SystemExit(f"{name}: vector accumulator is not updated in place")
    if re.search(rf"{mnemonic} .*\(%rbp\)", current):
        raise SystemExit(f"{name}: vector accumulator still loads from its stack slot")
    if not re.search(rf"{mnemonic} .*\(%rbp\)", disabled):
        raise SystemExit(f"{name}: kill-switch control did not restore stack accumulator")

# Sum loops need no vector temporary stack traffic at all. Dot/square loops may
# retain one product-input temporary, but the accumulator store must be gone.
for name in ("p15_sum_f32", "p16_sum_f64"):
    if re.search(r"vmov\w* %ymm\d+, -?\d+\(%rbp\)", body(new, name)):
        raise SystemExit(f"{name}: unexpected vector stack store")
PY
