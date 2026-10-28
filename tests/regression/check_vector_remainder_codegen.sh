#!/usr/bin/env bash
# Structural regression for the AVX2 vector transitions that divide a 64-bit
# byte induction variable back into a scalar index, and for the strided-2 i32
# sum-reduction vectorizer, whose scalar remainder starts from that same divide.
# Both are shape contracts: a scalar fallback would satisfy every arithmetic
# comparison in the corpus while leaving the transform dead.
set -euo pipefail
CCC=${CCC:-./target/release/lccc}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-vector-remainder.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
flags=(-O2 -march=x86-64-v3 -ffast-math -ffp-contract=fast)

"$CCC" "${flags[@]}" -S "$dir/vectorize_matmul_tail.c" -o "$tmp/matmul.s"
"$CCC" "${flags[@]}" -S "$dir/../benchmark/patterns/simd_fp_oracle.c" \
    -o "$tmp/reduction.s"

# The strided-2 i32 sum-reduction fixture: one function reading a global array
# (the whole vector prefix is provably inside the object) and one reading a
# run-time pointer (only the prefix inside the scalar loop's own bytes may be
# vectorized). Its output is compared against GCC, so the packed arithmetic
# assertion below cannot pass on a scalar fallback that happens to be correct.
strided_src="$dir/vectorize_strided_i32_reduce.c"
"$CCC" "${flags[@]}" -S "$strided_src" -o "$tmp/strided.s"
"$CCC" "${flags[@]}" "$strided_src" -o "$tmp/strided"
${GCC_BIN:-gcc} -O2 -march=x86-64-v3 "$strided_src" -o "$tmp/strided-ref"
expected_out=$("$tmp/strided-ref")
actual_out=$(timeout 60 "$tmp/strided")
if [[ $actual_out != "$expected_out" ]]; then
    printf 'FAIL: strided i32 reduction output differs from GCC:\n  lccc: %q\n  gcc : %q\n' \
        "$actual_out" "$expected_out" >&2
    exit 1
fi
python3 - "$tmp/strided.s" <<'PY2'
import re
import sys

text = open(sys.argv[1], encoding="utf-8").read()
packed = re.compile(r"\bvp(?:addd|subd|xor[ds]?|ord|andd|mulld|slld|srld|srad)\b")
for name in ("sum_global", "sum_dynamic"):
    match = re.search(rf"(?ms)^{name}:\n(.*?)^\.size {name},", text)
    if not match:
        raise SystemExit(f"{name}: missing assembly body")
    body = match.group(1)
    if "%ymm" not in body:
        raise SystemExit(
            f"{name}: strided-2 i32 reduction was not vectorized (no 256-bit register)"
        )
    if not packed.search(body):
        raise SystemExit(
            f"{name}: no packed dword arithmetic in the reduction loop "
            "(a scalar fallback cannot be the vectorized transform)"
        )
PY2

python3 - "$tmp/matmul.s" "$tmp/reduction.s" "$dir/lib" <<'PY'
import re
import sys

sys.path.insert(0, sys.argv[3])
from byte_iv_divide import divides_byte_iv  # noqa: E402

matmul = open(sys.argv[1], encoding="utf-8").read()
reduction = open(sys.argv[2], encoding="utf-8").read()


def body(text, name):
    match = re.search(rf"(?ms)^{re.escape(name)}:\n(.*?)^\.size {re.escape(name)},", text)
    if not match:
        raise SystemExit(f"missing assembly body for {name}")
    return match.group(1)


# matmul_verify is a single-call-site static and folds into main at -O2
# (GCC's -finline-functions-called-once behavior); the vector loop therefore
# lives in whichever body absorbed it.  Search matmul_verify first and fall
# back to the whole translation unit.
try:
    matmul_body = body(matmul, "matmul_verify")
except SystemExit:
    matmul_body = matmul
if "vfmadd231pd" not in matmul_body:
    raise SystemExit("matmul_verify: AVX2 vector loop was not emitted")
if re.search(r"\bidiv[lq]?\b", matmul_body):
    raise SystemExit("matmul_verify: signed division remains in the scalar-tail transition")
if not divides_byte_iv(matmul_body, 3):
    raise SystemExit("matmul_verify: missing byte-IV to F64-index divide")

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
    if not divides_byte_iv(current, shift):
        raise SystemExit(f"{name}: missing byte-IV remainder divide by {shift}")
PY
