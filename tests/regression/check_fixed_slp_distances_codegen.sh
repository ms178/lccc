#!/usr/bin/env bash
# Profitable complete fixed vectors pack; strict, kill-switch, and partial-width
# controls retain scalar source operations.
set -euo pipefail
CCC=${CCC:-./target/release/lccc}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-fixed-slp.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
base=(-O3 -march=x86-64-v3)
fast=(-ffast-math -ffp-contract=fast)

"$CCC" "${base[@]}" -ffp-contract=off -S "$dir/fixed_slp_distances.c" -o "$tmp/strict.s"
"$CCC" "${base[@]}" "${fast[@]}" -S "$dir/fixed_slp_distances.c" -o "$tmp/fast.s"
CCC_NO_FIXED_SLP=1 "$CCC" "${base[@]}" "${fast[@]}" -S \
    "$dir/fixed_slp_distances.c" -o "$tmp/control.s"
# Baseline x86-64 has no AVX.  Fast FP semantics alone must not introduce a
# 256-bit instruction into an otherwise scalar function.
"$CCC" -O3 "${fast[@]}" -S "$dir/fixed_slp_distances.c" -o "$tmp/baseline.s"

python3 - "$tmp/strict.s" "$tmp/fast.s" "$tmp/control.s" \
    "$tmp/baseline.s" <<'PY'
import re
import sys
strict, fast, control, baseline = [
    open(p, encoding="utf-8").read() for p in sys.argv[1:]
]

def body(text, name):
    m = re.search(rf"(?ms)^{name}:\n(.*?)^\.size {name},", text)
    if not m:
        raise SystemExit(f"missing {name}")
    return m.group(1)

def insns(text):
    return sum(1 for line in text.splitlines()
               if (s := line.strip()) and not s.startswith(('.', '#'))
               and not s.endswith(':') and not s.startswith('.cfi'))

for name, load, sub, mul in (
    ("distance8_f32", "vmovups", "vsubps", "vmulps"),
    ("distance4_f64", "vmovupd", "vsubpd", "vmulpd"),
):
    packed = body(fast, name)
    for op in (load, sub, mul):
        if op not in packed:
            raise SystemExit(f"{name}: missing {op}")
    if "%ymm" not in packed:
        raise SystemExit(f"{name}: expected full 256-bit SLP")
    if "%ymm" in body(strict, name):
        raise SystemExit(f"{name}: strict FP unexpectedly reassociated")
    if "%ymm" in body(control, name):
        raise SystemExit(f"{name}: CCC_NO_FIXED_SLP did not restore scalar code")
    if re.search(r"vmovdqu\s+%ymm\d+,\s*-?\d+\(%(?:r|e)(?:sp|bp)\)", packed):
        raise SystemExit(f"{name}: register SLP web spilled")

# F32x8 is the clear static profitability case.
if insns(body(fast, "distance8_f32")) >= insns(body(control, "distance8_f32")):
    raise SystemExit("distance8_f32: SLP did not reduce instruction count")

# Narrow/partial objects must neither lose profitability nor be over-read.
for name in ("distance4_f32", "distance3_f64"):
    if "%ymm" in body(fast, name):
        raise SystemExit(f"{name}: partial/narrow vector should remain scalar")

# The fold sinks its packed loads to the return.  Calls/stores/atomics between
# the source loads and return therefore make the transform illegal.
if "%ymm" in body(fast, "distance8_before_call"):
    raise SystemExit("distance8_before_call: loads moved across aliasing call")

# AVX is not part of the baseline x86-64 ISA.
for name in ("distance8_f32", "distance4_f64"):
    if "%ymm" in body(baseline, name):
        raise SystemExit(f"{name}: baseline target unexpectedly uses AVX")

PY
