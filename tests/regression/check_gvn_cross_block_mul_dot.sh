#!/usr/bin/env bash
# Cross-block integer-Mul CSE x dot-product coexistence gate.
#
# WHAT THIS PINS
#
#   tests/regression/gvn_cross_block_mul_dot_product.c carries two
#   multiplies in one loop: a VARIANT `i * i` (the header exit compare,
#   duplicated by the body's `t += i * i`) and a dot-product
#   `a[i] * b[i]` feeding an accumulator through body-local loads.
#
#   The cross-block GVN rename must collapse the variant duplicate onto
#   the header's canonical Mul — leaving exactly ONE register-register
#   `imul` for `i*i` in the whole function — while the dot-product Mul
#   keeps its body-local definition with LOAD operands (memory-form
#   imul). Renaming the dot-product Mul away would break the vectorizer's
#   body-local structural requirement; the IR-level unit test
#   (test_cross_block_cse_preserves_dot_product) pins the value-numbering
#   argument, this gate pins the end-to-end pipeline result.
#
# FAILURE MODES THIS CATCHES
#
#   * cross-block Mul CSE disabled or no longer firing: the body
#     re-materializes `i * i` (extra register-register imuls in the loop
#     body — the pre-PR shape had FOUR imuls total, two of them in the
#     body);
#   * the rename over-reaching into the dot-product Mul: the memory-form
#     `imul (mem), reg` disappears or turns register-register;
#   * the header's canonical exit-compare Mul being deleted (phi-feed /
#     exit-shape guards): the loop head loses its `imul`+`cmp` pair.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
src="$dir/gvn_cross_block_mul_dot_product.c"
tmp=$(mktemp -d "${TMPDIR:-}/tmp/lccc-gvn-xblk-dot.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

"$CCC" -O2 -S "$src" -o "$tmp/out.s"

python3 - "$tmp/out.s" <<'PY'
import re
import sys

text = open(sys.argv[1], encoding="utf-8").read()

match = re.search(r"(?ms)^dot_and_variant:\n(.*?)^\.size dot_and_variant,", text)
if not match:
    raise SystemExit("missing assembly body for dot_and_variant")
body = match.group(1)

imuls = re.findall(r"^\s*imul[a-z]*\s+(.+)$", body, re.M)
if not imuls:
    raise SystemExit("no imuls emitted: both the variant and dot Muls vanished")

reg_reg = [m for m in imuls if re.fullmatch(r"(%\w+),\s*%(\w+)", m.strip())]
mem_form = [m for m in imuls if re.search(r"^\(", m.strip()) or re.search(r",\s*\(", m)]

if len(mem_form) < 1:
    raise SystemExit(
        f"dot-product Mul lost its memory-form load operand: {[m.strip() for m in imuls]}"
    )

if len(reg_reg) != 1:
    raise SystemExit(
        "expected exactly ONE register-register imul (the header's canonical "
        f"i*i exit compare), found {len(reg_reg)}: {[m.strip() for m in reg_reg]} — "
        "the cross-block rename is either not firing (duplicates survive) or "
        "deleting the canonical (over-reach)"
    )

if len(imuls) != 2:
    raise SystemExit(
        f"expected exactly 2 imuls (canonical variant + dot-product), found {len(imuls)}: "
        f"{[m.strip() for m in imuls]}"
    )

print("gvn-cross-block-mul-dot: ok "
      f"(1 canonical i*i imul + 1 memory-form dot imul)")
PY
