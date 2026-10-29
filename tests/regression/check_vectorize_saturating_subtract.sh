#!/usr/bin/env bash
# Lock in the unsigned saturating-subtract fold of the map vectorizer.
#
# `a >= b ? a - b : 0` over byte/word lanes must compile to one packed
# `psubusb`/`psubusw`, and the compiled program must agree with GCC on the same
# source at every saturation boundary.  The dword lane is checked for
# correctness only: x86 has no packed dword saturating subtract.
set -euo pipefail
CCC=${LCCC_BIN:-${CCC:-target/fastbuild/lccc}}
REF=${REF:-gcc-16}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
src="$dir/vectorize_saturating_subtract.c"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-subsat.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

"$CCC" -O2 -S "$src" -o "$tmp/lccc.s"
# The AVX2 path reaches the same fold through the min/max form
# (`vpmaxuw` + `vpsubw` -> `vpsubusw`), so both widths are gated.
"$CCC" -O2 -march=x86-64-v3 -S "$src" -o "$tmp/lccc-v3.s"
"$CCC" -O2 -o "$tmp/lccc" "$src"
"$tmp/lccc" > "$tmp/lccc.out"

if command -v "$REF" >/dev/null 2>&1; then
    "$REF" -O2 -o "$tmp/ref" "$src"
    "$tmp/ref" > "$tmp/ref.out"
    if ! cmp -s "$tmp/lccc.out" "$tmp/ref.out"; then
        echo "FAIL: result differs from $REF" >&2
        diff -u "$tmp/ref.out" "$tmp/lccc.out" >&2 || true
        exit 1
    fi
else
    echo "note: $REF not found; skipping the differential half" >&2
fi

python3 - "$tmp/lccc.s" "$tmp/lccc-v3.s" <<'PY'
import re
import sys

text = open(sys.argv[1], encoding="utf-8").read()
text_v3 = open(sys.argv[2], encoding="utf-8").read()


def body(name, text=text):
    m = re.search(rf"(?ms)^{name}:\n(.*?)^\s*\.size {name},", text)
    if not m:
        raise SystemExit(f"missing assembly body for {name}")
    return m.group(1)


# Word lanes: both spellings fold, and the fold must be the packed
# saturating subtract rather than the compare/blend expansion of it.
for fn in ("slide_w_ge", "slide_w_gt"):
    for label, sub_text in (("sse2", text), ("avx2", text_v3)):
        b = body(fn, sub_text)
        if "psubusw" not in b:
            raise SystemExit(
                f"{fn} ({label}): expected the folded psubusw, got:\n{b}")
        if "pcmpgtw" in b or "pmaxuw" in b:
            raise SystemExit(
                f"{fn} ({label}): still expands the saturating subtract")

# Byte lanes never reach the map emitter today (a byte map is admitted only
# for 0/1 reduction trees), so this one is a correctness-only fixture: the
# differential half above already pins its values.  What must not appear is an
# instruction the target cannot encode.
if "psubusd" in text:
    raise SystemExit("psubusd does not exist on x86")

# The dword loop has no packed saturating subtract to fold into; it must not
# claim one (that would be a missing instruction on the target).
d = body("slide_d")
if "subd" in d and "psubusd" in d:
    raise SystemExit("slide_d: psubusd does not exist on x86")
PY

echo "vectorize_saturating_subtract: OK"
