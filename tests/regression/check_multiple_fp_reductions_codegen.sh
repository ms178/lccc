#!/usr/bin/env bash
# Four independent scalar FP reductions stay in four XMM accumulators.
set -euo pipefail
CCC=${CCC:-./target/release/lccc}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-multi-fp-red.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
flags=(-O3 -march=x86-64-v3 -ffast-math)

"$CCC" "${flags[@]}" -S "$dir/multiple_fp_reductions.c" -o "$tmp/new.s"
CCC_NO_FP_COPY_WEB=1 "$CCC" "${flags[@]}" -S \
    "$dir/multiple_fp_reductions.c" -o "$tmp/control.s"

python3 - "$tmp/new.s" "$tmp/control.s" <<'PY'
import re
import sys

new = open(sys.argv[1], encoding="utf-8").read()
control = open(sys.argv[2], encoding="utf-8").read()


def body(text, name):
    match = re.search(rf"(?ms)^{name}:\n(.*?)^\.size {name},", text)
    if not match:
        raise SystemExit(f"missing assembly body for {name}")
    return match.group(1)


for name, mnemonic in (("four_f32", "vaddss"), ("four_f64", "vaddsd")):
    current = body(new, name)
    disabled = body(control, name)
    direct = re.findall(
        rf"{mnemonic}\s+[^\n]*\([^\n]*\),\s*%(xmm\d+),\s*%\1", current
    )
    if len(set(direct)) < 4:
        raise SystemExit(f"{name}: expected four distinct in-place XMM accumulators")
    if re.search(r"movs[sd]\s+%xmm\d+,\s*-?\d+\(%(?:r|e)(?:sp|bp)\)", current):
        raise SystemExit(f"{name}: loop-carried FP accumulator still spills")
    if not re.search(r"movs[sd]\s+%xmm\d+,\s*-?\d+\(%(?:r|e)(?:sp|bp)\)", disabled):
        raise SystemExit(f"{name}: kill-switch control did not restore stack homes")
PY
