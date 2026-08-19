#!/usr/bin/env bash
# Contract-sensitive AVX2 dot reduction: fused memory step only under fast contraction.
set -euo pipefail
CCC=${CCC:-./target/release/lccc}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
src="$dir/../benchmark/patterns/simd_fp_oracle.c"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-dot-fma.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
common=(-O3 -march=x86-64-v3 -fassociative-math)

"$CCC" "${common[@]}" -ffp-contract=fast -S "$src" -o "$tmp/fast.s"
"$CCC" "${common[@]}" -ffp-contract=off -S "$src" -o "$tmp/off.s"

python3 - "$tmp/fast.s" "$tmp/off.s" <<'PY'
import re
import sys

fast = open(sys.argv[1], encoding="utf-8").read()
off = open(sys.argv[2], encoding="utf-8").read()


def body(text, name):
    match = re.search(rf"(?ms)^{re.escape(name)}:\n(.*?)^\.size {re.escape(name)},", text)
    if not match:
        raise SystemExit(f"missing assembly body for {name}")
    return match.group(1)


for name, suffix in (("p17_dot_f32", "ps"), ("p18_dot_f64", "pd")):
    fused = body(fast, name)
    separate = body(off, name)
    if not re.search(rf"vfmadd231{suffix}\s+\([^\n]+\),\s*%ymm0,\s*%ymm\d+", fused):
        raise SystemExit(f"{name}: missing memory-folded packed FMA")
    if re.search(r"vmovdqu\s+%ymm0,\s*-?\d+\(%(?:r|e)bp\)", fused):
        raise SystemExit(f"{name}: transient vector product still spills")
    if re.search(rf"vfmadd\w*{suffix}\b", separate):
        raise SystemExit(f"{name}: packed FMA emitted with -ffp-contract=off")
    mul = "vmulps" if suffix == "ps" else "vmulpd"
    add = "vaddps" if suffix == "ps" else "vaddpd"
    if mul not in separate or add not in separate:
        raise SystemExit(f"{name}: contract-off control lost separate multiply/add")
PY
