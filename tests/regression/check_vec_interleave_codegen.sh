#!/usr/bin/env bash
# Structural regression for vec_interleave: latency-bound vector reduction
# loops must be split into four independent accumulator chains with
# displacement-folded memory operands (ICX's vfmadd231pd YMM style), and the
# kill switch must restore the single-chain form.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source_file="$dir/../benchmark/patterns/simd_fp_oracle.c"
out=$(mktemp "${TMPDIR:-/tmp}/lccc-vec-il.XXXXXX.s")
control=$(mktemp "${TMPDIR:-/tmp}/lccc-vec-il-ctl.XXXXXX.s")
trap 'rm -f "$out" "$control"' EXIT

"$CCC" -O3 -march=x86-64-v3 -ffast-math -ffp-contract=fast \
    -S "$source_file" -o "$out"
CCC_NO_VEC_INTERLEAVE=1 \
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


for name, mnemonic in {
    "p17_dot_f32": "vfmadd231ps",
    "p18_dot_f64": "vfmadd231pd",
}.items():
    current = body(new, name)
    disabled = body(old, name)
    # Four independent chains, three of them displacement-folded:
    #   vfmadd231pd 32(%rsi,%rax), %ymm1, %ymm5  (disp 32/64/96)
    folded = re.findall(rf"{mnemonic} (?:-?\d+)\(%r\w+,%r\w+\), %ymm\d+, %ymm\d+", current)
    if len(folded) < 3:
        raise SystemExit(
            f"{name}: expected >= 3 displacement-folded {mnemonic} chains, got {folded}"
        )
    if not re.search(rf"{mnemonic} \(%r\w+,%r\w+\), %ymm\d+, %ymm\d+", current):
        raise SystemExit(f"{name}: missing base (disp-0) chain")
    if len(re.findall(rf"{mnemonic}", current)) < 4:
        raise SystemExit(f"{name}: fewer than four {mnemonic} chains in the loop")
    # The kill-switch control must be back to a single chain without the
    # interleave mask (andl $-128 / andl $-64 style group alignment).
    if len(re.findall(rf"{mnemonic}", disabled)) > 2:
        raise SystemExit(f"{name}: kill-switch control still shows multiple chains")
    if re.search(r"andl \$-(?:64|128|256), %r\w+d", disabled):
        raise SystemExit(f"{name}: kill-switch control still carries the interleave mask")
PY
