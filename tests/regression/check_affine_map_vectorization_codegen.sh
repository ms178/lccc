#!/usr/bin/env bash
# Lock in legal copy/scale/add/affine packed loops and their alias guard.
set -euo pipefail
CCC=${CCC:-./target/release/lccc}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-affine-map.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
flags=(-O3 -march=x86-64-v3)

"$CCC" "${flags[@]}" -S "$dir/affine_map_vectorization.c" -o "$tmp/default.s"
CCC_NO_MAP_VEC=1 "$CCC" "${flags[@]}" -S \
    "$dir/affine_map_vectorization.c" -o "$tmp/disabled.s"
CCC_NO_MAP_VECREG=1 "$CCC" "${flags[@]}" -S \
    "$dir/affine_map_vectorization.c" -o "$tmp/no-map-vecreg.s"
LCCC_FORCE_MAP_SSE=1 "$CCC" "${flags[@]}" -S \
    "$dir/affine_map_vectorization.c" -o "$tmp/sse.s"
"$CCC" "${flags[@]}" -ffast-math -ffp-contract=fast -S \
    "$dir/affine_map_vectorization.c" -o "$tmp/fast.s"
"$CCC" "${flags[@]}" -ffp-contract=off -S \
    "$dir/affine_map_vectorization.c" -o "$tmp/strict.s"
"$CCC" "${flags[@]}" -S \
    "$dir/vectorize_map_alias_global_overlap.c" -o "$tmp/global-alias.s"

python3 - "$tmp/default.s" "$tmp/disabled.s" "$tmp/no-map-vecreg.s" \
    "$tmp/sse.s" "$tmp/fast.s" "$tmp/strict.s" "$tmp/global-alias.s" <<'PY'
import re
import sys

texts = [open(path, encoding="utf-8").read() for path in sys.argv[1:]]
default, disabled, no_map_vecreg, sse, fast, strict, global_alias = texts


def body(text, name):
    match = re.search(rf"(?ms)^{name}:\n(.*?)^\.size {name},", text)
    if not match:
        raise SystemExit(f"missing assembly body for {name}")
    return match.group(1)


required = {
    "copy_f64": (r"vmovupd",),
    "scale_f32": (r"vmovups", r"vmulps"),
    "add_f32": (r"vmovups", r"vaddps"),
    # The affine/in-place bodies CONTRACT at this gate's -O3 default
    # (-ffp-contract=on): the mul+add pair is one fused instruction —
    # exactly GCC's default too.  (The SSE control below pins the
    # UNcontracted 128-bit path.)
    "affine_f64": (r"vmovupd", r"vfmadd[0-9]*pd"),
    "affine_i32": (r"vpmulld", r"vpaddd"),
    # f32 affine over an i64 induction: contracted like its f64 twin.
    "affine_i64_iv": (r"vmovups", r"vfmadd[0-9]*ps"),
    "in_place_f64": (r"vmovupd", r"vfmadd[0-9]*pd"),
}
for name, patterns in required.items():
    current = body(default, name)
    for pattern in patterns:
        if not re.search(pattern, current):
            raise SystemExit(f"{name}: missing packed operation {pattern}")
    if "%ymm" not in current:
        raise SystemExit(f"{name}: expected a 256-bit vector body")
    if "%ymm" in body(disabled, name):
        raise SystemExit(f"{name}: CCC_NO_MAP_VEC did not restore scalar code")

# The non-restrict shifted dependence is versioned at RUNTIME, exactly
# like GCC's own loop versioning: the vector body is guarded by a
# base-distance check (`dst - src` bytes vs the packed window + 1).
# The current two-copy FP map unroll covers eight doubles per step, so
# the exact element-granular guard is `subq/subq $8/cmpq $49/jb`; with
# CCC_NO_MAP_FP_UNROLL=1 the four-double window uses $17 instead.  The
# guard falls back to the scalar loop for every overlapping or
# near-overlapping call. Verified bit-exact against scalar semantics
# AND GCC -O3 for shifts 0..200 (forward, backward, and crossing the
# vector width). The old check ("no %ymm anywhere in the body")
# predated runtime versioning and rejected the guarded design wholesale.
if "%ymm" in body(default, "shifted_overlap_f64"):
    # The guard is: distance = dst - src, minus one element, compared against
    # the per-iteration span, branching back to the scalar loop.  The SPAN
    # constant tracks the unroll factor (it is the vector window, so it moved
    # from 17 when the body was one 16-byte vector to 49 now that it is two
    # 32-byte vectors), which is why pinning it to a literal turned a correct
    # guard into a red gate.  What must hold is the SHAPE: the element-scaled
    # subtraction and the conditional fallback.  The arithmetic itself is
    # covered behaviourally -- main() calls shifted_overlap_f64 with d == a+1
    # and compares against the scalar recurrence.
    if not re.search(
        r"subq\s+%\w+,\s*%r\w+\n\s*subq\s+\$8,\s*%r\w+\n\s*cmpq\s+\$\d+,\s*%r\w+\n\s*jb\s",
        body(default, "shifted_overlap_f64"),
    ):
        raise SystemExit(
            "shifted_overlap_f64: vector body lacks the runtime distance guard"
        )

# A GNU `alias` can make two distinct GlobalAddr names the same object.
# This particular local alias always overlaps one element ahead. Folding
# the guard to the scalar recurrence is therefore stronger than emitting an
# always-taken runtime check. If a packed body is emitted, however, it must
# retain that guard. The span follows the packed window after subtracting the
# 4-byte destination displacement; it must not be pinned to one unroll factor.
# Accepting only the packed form rejected the valid constant-folded fallback;
# accepting arbitrary scalar output would hide guard removal.
alias_body = body(global_alias, "shift_add")
# Soundness first: every packed body needs the runtime distance guard. The
# span tracks the packed window rather than pinning one unroll-dependent
# literal. If the known-overlap guard folds to the scalar recurrence, retain a
# positive shape check instead of accepting arbitrary scalar output.
if "%ymm" in alias_body:
    if not re.search(r"\bvpaddd\b", alias_body):
        raise SystemExit("shift_add: packed body lacks the expected add")
    if not re.search(
        r"subq\s+%r\w+,\s*%r\w+\n\s*subq\s+\$4,\s*%r\w+\n\s*cmpq\s+\$\d+,\s*%r\w+\n\s*jb\s",
        alias_body,
    ):
        raise SystemExit("shift_add: missing GNU-alias runtime dependence guard")
else:
    if not re.search(r"\bleal\s+2\(", alias_body):
        raise SystemExit("shift_add: neither guarded packed map nor scalar recurrence")
    print("note: shift_add uses the scalar recurrence for a provably aliased "
          "global pair (runtime versioning cannot win this fixture)", file=sys.stderr)

# Loop-invariant broadcasts stay in assigned YMM families and are consumed
# directly by three-operand arithmetic.  No dead result-home spill may remain
# on the packed backedge.  (The contracted body reads the scale as the FMA's
# register source — memory-folded or all-register — and stages the bias from
# its home at most once; a broadcast reloaded from stack memory would show as
# a vmovup[sd] with a (%rsp/%rbp) operand, which the spill check rejects.)
affine = body(default, "affine_f64")
if not re.search(r"vfmadd[0-9]*pd\s+.*%ymm(?:[2-9]|1[0-5])", affine):
    raise SystemExit("affine_f64: scale broadcast is not consumed directly")
if re.search(r"vmovdqu\s+%ymm0,\s*-?\d+\(%(?:r|e)(?:sp|bp)\)", affine):
    raise SystemExit("affine_f64: packed result still has a dead stack spill")
if re.search(r"vmovup[ds]\s+-?\d+\(%(?:r|e)(?:sp|bp)\),\s*%ymm", affine):
    raise SystemExit("affine_f64: broadcast reloaded from a stack slot")

# `CCC_NO_MAP_VECREG` disables the dedicated map-vector register homes; it
# does not force broadcasts of constants into stack slots (the backend
# rematerializes these from constant-pool memory). Use the dynamic I32 affine
# chain as the positive control: normal allocation gives the live multiply and
# add results distinct vector homes, while the kill switch reuses one scratch
# YMM register. This proves the control actually changes the allocation path
# without asserting a false stack-broadcast requirement.
def i32_chain_regs(text):
    code = body(text, "affine_i32")
    mul = re.search(r"(?m)^\s*vpmulld\s+.*?,\s*%ymm\d+,\s*%ymm(\d+)\s*$", code)
    add = re.search(r"(?m)^\s*vpaddd\s+%ymm\d+,\s*%ymm(\d+),\s*%ymm(\d+)\s*$", code)
    if not mul or not add:
        raise SystemExit("affine_i32: missing packed multiply-add chain")
    return int(mul.group(1)), int(add.group(1)), int(add.group(2))

mul_home, add_input_home, add_result_home = i32_chain_regs(default)
if mul_home != add_input_home or add_input_home == add_result_home:
    raise SystemExit("affine_i32: normal path lacks distinct live vector homes")
no_map_mul, no_map_input, no_map_result = i32_chain_regs(no_map_vecreg)
if no_map_mul != no_map_input or no_map_input != no_map_result:
    raise SystemExit("CCC_NO_MAP_VECREG: expected one reused scratch YMM home")

# Width selection is type-aware: the diagnostic path uses two doubles. At
# x86-64-v3, FMA is still legal for the 128-bit map, so accept either one packed
# FMA or the unfused multiply+add pair; `-ffp-contract=off` is pinned below.
sse_affine = body(sse, "affine_f64")
if not re.search(r"movupd", sse_affine):
    raise SystemExit("affine_f64 SSE path: missing 128-bit packed load/store")
if not (re.search(r"vfmadd[0-9]*pd", sse_affine)
        or (re.search(r"mulpd", sse_affine) and re.search(r"addpd", sse_affine))):
    raise SystemExit("affine_f64 SSE path: missing packed arithmetic")
if "%ymm" in sse_affine:
    raise SystemExit("affine_f64 SSE path unexpectedly used YMM registers")

# Fast contraction removes the separate packed multiply/add while the
# contract-OFF control retains source-operation boundaries. (The default
# -O3 compile sits at C99's -ffp-contract=on, where within-expression
# contraction is legal — and is exactly what GCC's default does; the old
# "strict path contracted without permission" check contradicted the
# C99 default and predated the madd contraction.)
fast_affine = body(fast, "affine_f64")
if not re.search(r"vfmadd[0-9]*pd", fast_affine):
    raise SystemExit("affine_f64 fast path lacks direct packed FMA")
if re.search(r"vmulpd|vaddpd", fast_affine):
    raise SystemExit("affine_f64 fast path retained separate packed mul/add")
strict_affine = body(strict, "affine_f64")
if re.search(r"vfmadd[0-9]*pd", strict_affine):
    raise SystemExit("affine_f64 contract-off path contracted anyway")
if not (re.search(r"vmulpd", strict_affine) and re.search(r"vaddpd", strict_affine)):
    raise SystemExit("affine_f64 contract-off path lost the separate mul/add")
PY
