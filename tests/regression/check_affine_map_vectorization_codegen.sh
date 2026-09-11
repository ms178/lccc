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
    "$dir/affine_map_vectorization.c" -o "$tmp/stack-broadcast.s"
LCCC_FORCE_MAP_SSE=1 "$CCC" "${flags[@]}" -S \
    "$dir/affine_map_vectorization.c" -o "$tmp/sse.s"
"$CCC" "${flags[@]}" -ffast-math -ffp-contract=fast -S \
    "$dir/affine_map_vectorization.c" -o "$tmp/fast.s"

python3 - "$tmp/default.s" "$tmp/disabled.s" "$tmp/stack-broadcast.s" \
    "$tmp/sse.s" "$tmp/fast.s" <<'PY'
import re
import sys

texts = [open(path, encoding="utf-8").read() for path in sys.argv[1:]]
default, disabled, stack_broadcast, sse, fast = texts


def body(text, name):
    match = re.search(rf"(?ms)^{name}:\n(.*?)^\.size {name},", text)
    if not match:
        raise SystemExit(f"missing assembly body for {name}")
    return match.group(1)


required = {
    "copy_f64": (r"vmovupd",),
    "scale_f32": (r"vmovups", r"vmulps"),
    "add_f32": (r"vmovups", r"vaddps"),
    "affine_f64": (r"vmovupd", r"vmulpd", r"vaddpd"),
    "affine_i32": (r"vpmulld", r"vpaddd"),
    "affine_i64_iv": (r"vmovups", r"vmulps", r"vaddps"),
    "in_place_f64": (r"vmovupd", r"vmulpd", r"vaddpd"),
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

# The non-restrict shifted dependence must not be widened.
if "%ymm" in body(default, "shifted_overlap_f64"):
    raise SystemExit("shifted_overlap_f64: unsafe overlapping loop was vectorized")

# Loop-invariant broadcasts stay in assigned YMM families and are consumed
# directly by three-operand arithmetic.  No dead result-home spill may remain
# on the packed backedge.
affine = body(default, "affine_f64")
if not re.search(r"vmulpd\s+%ymm\d+,\s*%ymm0,\s*%ymm0", affine):
    raise SystemExit("affine_f64: scale broadcast is not consumed directly")
if not re.search(r"vaddpd\s+%ymm\d+,\s*%ymm0,\s*%ymm0", affine):
    raise SystemExit("affine_f64: bias broadcast is not consumed directly")
if re.search(r"vmovdqu\s+%ymm0,\s*-?\d+\(%(?:r|e)(?:sp|bp)\)", affine):
    raise SystemExit("affine_f64: packed result still has a dead stack spill")

# The scoped register-allocation kill switch must retain the mature stack-home
# path, proving the direct-register assertion above is not a vacuous matcher.
stack_affine = body(stack_broadcast, "affine_f64")
if not re.search(r"vmulpd\s+-?\d+\(%(?:r|e)(?:sp|bp)\)", stack_affine):
    raise SystemExit("affine_f64: CCC_NO_MAP_VECREG control lacks stack broadcast")

# Width selection is type-aware: the diagnostic 128-bit path uses two doubles,
# while preserving the same expression family.
sse_affine = body(sse, "affine_f64")
for pattern in (r"movupd", r"mulpd", r"addpd"):
    if not re.search(pattern, sse_affine):
        raise SystemExit(f"affine_f64 SSE path: missing {pattern}")
if "%ymm" in sse_affine:
    raise SystemExit("affine_f64 SSE path unexpectedly used YMM registers")

# Fast contraction removes the separate packed multiply/add while strict mode
# retains source-operation boundaries.
fast_affine = body(fast, "affine_f64")
if not re.search(r"vfmadd132pd\s+%ymm\d+,\s*%ymm\d+,\s*%ymm0", fast_affine):
    raise SystemExit("affine_f64 fast path lacks direct packed FMA")
if re.search(r"vmulpd|vaddpd", fast_affine):
    raise SystemExit("affine_f64 fast path retained separate packed mul/add")
if "vfmadd132pd" in affine:
    raise SystemExit("affine_f64 strict path contracted without permission")
PY
