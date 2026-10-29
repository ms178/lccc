#!/usr/bin/env bash
# Structural contract for unsigned reduction vectorization.
#
# The companion C battery (vectorize_unsigned_reduction.c) proves the numeric
# answers; this script pins the codegen shape that produces them, at both the
# SSE2 baseline (-O2) and the AVX2 level (-march=x86-64-v3):
#
#   * same-width unsigned sums/dots vectorize with packed integer ops and an
#     UNSIGNED loop-exit predicate (jb/jae), proving the Ult comparison was
#     carried through rather than silently lowered as signed Slt;
#   * the three soundness fences stay scalar — unsigned widening (no
#     zero-extending lane composite), unsigned guarded sums (no unsigned
#     masked composite) and inclusive Ule bounds (outside the strict grammar).
set -euo pipefail
CCC=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-unsigned-reduction.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

"$CCC" -O2 -S "$dir/vectorize_unsigned_reduction.c" -o "$tmp/sse2.s"
"$CCC" -O2 -march=x86-64-v3 -S "$dir/vectorize_unsigned_reduction.c" -o "$tmp/avx2.s"

python3 - "$tmp/sse2.s" "$tmp/avx2.s" <<'PY'
import re
import sys


def body(text, name):
    match = re.search(
        rf"(?ms)^{re.escape(name)}:\n(.*?)^\.size {re.escape(name)},", text
    )
    if not match:
        raise SystemExit(f"missing assembly body for {name}")
    return match.group(1)


sse2, avx2 = (open(p, encoding="utf-8").read() for p in sys.argv[1:])

# Functions that must vectorize, with a packed integer op that actually
# processes lanes (plain movdqu alone would not prove reduction work moved
# into the vector domain).
must_vectorize = {
    "sum_u32_u64iv": (r"[vp]*paddd", r"[vp]*paddd"),
    "sum_u32_u32iv": (r"[vp]*paddd", r"[vp]*paddd"),
    "sum_u64": (r"[vp]*paddq", r"[vp]*paddq"),
    "dot_u32": (r"pmul(?:ld|udq)", r"vpmulld"),
    "dot_u64": (r"[vp]*paddq", r"[vp]*paddq"),
}
for name, (sse2_pat, avx2_pat) in must_vectorize.items():
    for text, pat, level in (
        (sse2, sse2_pat, "SSE2"),
        (avx2, avx2_pat, "AVX2"),
    ):
        b = body(text, name)
        if not re.search(pat, b):
            raise SystemExit(f"{name} not vectorized at {level}: {pat}")
        # The vector loop's exit must keep the source's unsigned predicate:
        # require the below/above-or-equal family and reject signed compares.
        if not re.search(r"\bj(?:b|ae)\b", b):
            raise SystemExit(f"{name} lost its unsigned exit predicate at {level}")
        if re.search(r"\bj(?:l|ge|g|le)\b", b):
            raise SystemExit(f"{name} gained a signed exit predicate at {level}")

# Functions that must stay scalar (no packed integer vector mnemonic at all).
must_stay_scalar = ("widen_u32_to_u64", "guarded_u32", "sum_u32_le")
for name in must_stay_scalar:
    for text, level in ((sse2, "SSE2"), (avx2, "AVX2")):
        b = body(text, name)
        if re.search(r"[vp]?(?:padd|pmul|psub|por|pand|pxor)", b):
            raise SystemExit(f"{name} unexpectedly vectorized at {level}")

# The signed control kernel keeps its existing packed form.
for text, level, pat in (
    (sse2, "SSE2", r"paddd"),
    (avx2, "AVX2", r"vpaddd"),
):
    if not re.search(pat, body(text, "sum_i32")):
        raise SystemExit(f"sum_i32 regressed at {level}")

print("unsigned reduction codegen contract OK")
PY
