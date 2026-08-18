#!/usr/bin/env bash
# Structural regression for division-free AVX2 vector remainder transitions.
set -euo pipefail
CCC=${CCC:-./target/release/lccc}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-vector-remainder.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
flags=(-O2 -march=x86-64-v3 -ffast-math -ffp-contract=fast)

"$CCC" "${flags[@]}" -S "$dir/vectorize_matmul_tail.c" -o "$tmp/matmul.s"
"$CCC" "${flags[@]}" -S "$dir/../benchmark/patterns/simd_fp_oracle.c" \
    -o "$tmp/reduction.s"

python3 - "$tmp/matmul.s" "$tmp/reduction.s" <<'PY'
import re
import sys

matmul = open(sys.argv[1], encoding="utf-8").read()
reduction = open(sys.argv[2], encoding="utf-8").read()


def body(text, name):
    match = re.search(rf"(?ms)^{re.escape(name)}:\n(.*?)^\.size {re.escape(name)},", text)
    if not match:
        raise SystemExit(f"missing assembly body for {name}")
    return match.group(1)


matmul_body = body(matmul, "matmul_verify")
if "vfmadd231pd" not in matmul_body:
    raise SystemExit("matmul_verify: AVX2 vector loop was not emitted")
if re.search(r"\bidiv[lq]?\b", matmul_body):
    raise SystemExit("matmul_verify: signed division remains in the scalar-tail transition")
if not re.search(r"\bshrl?\s+\$3", matmul_body):
    raise SystemExit("matmul_verify: missing byte-IV to F64-index shift")

for name, shift, packed_op in (
    ("p15_sum_f32", 2, "vaddps"),
    ("p16_sum_f64", 3, "vaddpd"),
    ("p17_dot_f32", 2, "vfmadd231ps"),
    ("p18_dot_f64", 3, "vfmadd231pd"),
):
    current = body(reduction, name)
    if packed_op not in current:
        raise SystemExit(f"{name}: vector reduction was not emitted")
    if re.search(r"\bidiv[lq]?\b", current):
        raise SystemExit(f"{name}: signed division remains in the scalar-tail transition")
    if not re.search(rf"\bshrl?\s+\${shift}", current):
        raise SystemExit(f"{name}: missing byte-IV remainder shift by {shift}")
PY
