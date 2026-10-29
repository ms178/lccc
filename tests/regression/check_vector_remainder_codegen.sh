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

# Report the failing contract as a workflow annotation as well as on stderr.
# The lane wrapper truncates the log tail, so a bare `exit 1` from inside a
# heredoc is invisible in the PR; `check_ch_maj_codegen.sh` reports the same way.
fail() { # fail <title> <detail>
    printf '::error title=%s::%s\n' "$1" "$2" >&2
    printf 'FAIL %s: %s\n' "$1" "$2" >&2
    exit 1
}

"$CCC" "${flags[@]}" -S "$dir/vectorize_matmul_tail.c" -o "$tmp/matmul.s"
"$CCC" "${flags[@]}" -S "$dir/../benchmark/patterns/simd_fp_oracle.c" \
    -o "$tmp/reduction.s"

# The strided-2 i32 reduction fixture checks shared global and dynamic
# pointers, exact-width loads, a protected final gap, and the scalar tail.
# Compare arithmetic with GCC and require the packed path to remain live.
strided_src="$dir/vectorize_strided_i32_reduce.c"
CCC_TIME_PASSES=1 "$CCC" "${flags[@]}" -S "$strided_src" -o "$tmp/strided.s" 2>"$tmp/strided.passes"
if ! grep -Eq '\[PASS\] iter=0 vectorize_strided_reduce_pre_unroll: .*\([1-9][0-9]* changes\)' "$tmp/strided.passes"; then
    fail "strided i32 pre-unroll vectorization" \
        "the dedicated pre-unroll pass did not transform the regression fixture"
fi
"$CCC" "${flags[@]}" "$strided_src" -o "$tmp/strided"
${GCC_BIN:-gcc} -O2 -march=x86-64-v3 "$strided_src" -o "$tmp/strided-ref"
expected_out=$("$tmp/strided-ref")
actual_out=$(timeout 60 "$tmp/strided")
if [[ $actual_out != "$expected_out" ]]; then
    fail "strided i32 reduction arithmetic" \
        "output differs from GCC (lccc: $actual_out / gcc: $expected_out)"
fi

# Unlike guard-page bounds, a hardware read watchpoint observes a load of
# an intervening object even when that object is accessible. The driver first
# proves the watchpoint fires on a deliberate read, then runs the LCCC kernel.
"$CCC" "${flags[@]}" -c "$dir/fixtures/strided_exact/kernel.c" -o "$tmp/watch-kernel.o"
"$CCC" "${flags[@]}" -S "$dir/fixtures/strided_exact/kernel.c" -o "$tmp/watch-kernel.s"
${GCC_BIN:-gcc} -O2 "$dir/fixtures/strided_exact/watch_driver.c" "$tmp/watch-kernel.o" -o "$tmp/watch"
timeout 30 "$tmp/watch" || fail "strided exact-access watchpoint" "gap was read or watchpoint setup failed"
if ! grep -q 'vpinsrd' "$tmp/watch-kernel.s"; then
    fail "strided exact-access watchpoint" "kernel did not take the packed exact-load path"
fi
python3 - "$tmp/strided.s" <<'PY2'
import re
import sys

text = open(sys.argv[1], encoding="utf-8").read()
packed = re.compile(r"\bvp(?:addd|subd|xor[ds]?|ord|andd|mulld|slld|srld|srad)\b")
for name in ("sum_global_full", "sum_dynamic_tail", "sum_dynamic_full"):
    match = re.search(rf"(?ms)^{name}:\n(.*?)^\.size {name},", text)
    if not match:
        raise SystemExit(f"{name}: missing assembly body")
    body = match.group(1)
    def reject(reason):
        # One annotation with the body that failed, so the shape can be judged
        # without a compile of the revision that produced it.
        detail = body[:1500].replace("%", "%25").replace("\n", "%0A")
        print(f"::error title=strided-reduce {name}::{reason} --- {detail}",
              file=sys.stderr)
        raise SystemExit(f"{name}: {reason}")

    if "%ymm" not in body:
        reject("strided-2 i32 reduction was not vectorized (no 256-bit register)")
    if not packed.search(body):
        reject("no packed dword arithmetic in the reduction loop "
               "(a scalar fallback cannot be the vectorized transform)")
    if name == "sum_global_full":
        # The global stream reads a strong, locally binding definition whose
        # defined size the reducer proves (`ModuleSymbolFacts::global_min_bytes`),
        # so its loads widen to full 32-byte vectors: legal (every byte read is
        # inside the object) and cheaper than the strided assembly.  The wide
        # loads appear either materialised (`vmovdqu %ymm`) or memory-folded
        # into their consumers (a 32-byte `vpxor <mem>` stream is the fold of a
        # full wide load, which only contiguous loads qualify for).  What must
        # NOT come back is the pointer-limited strided form: the object proof
        # exists precisely so the shared stream stops paying for it.
        wide = len(re.findall(r"\bvmovdqu\b[^,]*, %ymm", body)) + len(
            re.findall(r"\bvpxor\s+-?\d*%\w+\(%\w+(?:,%\w+)?\)", body))
        if wide < 3:
            reject("sized global stream did not take the proven wide-load form")
    else:
        # Parameter streams have no provable extent, so they keep the
        # element-exact strided form: four dwords assembled by two vpinsrd
        # pairs and one vinserti128, never reading the inter-dword gaps.
        if len(re.findall(r"\bvpinsrd\b", body)) < 2 or "vinserti128" not in body:
            reject("shared/dynamic stream lacks the exact four-dword load")
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
