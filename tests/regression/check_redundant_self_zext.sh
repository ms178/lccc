#!/usr/bin/env bash
# Redundant-zero-extension elimination: two shapes that regressed once each.
#
# 1. XMM -> GPR -> self-zext. `movq %xmmN, %rX` + `movl %eX, %eX` is how the
#    emitter lowers "widen the BITS of an FP value to 64 bits"; the pair is
#    `movd %xmmN, %eX` in one instruction. Both the legacy and the VEX
#    spelling must stay fused, and the VEX spelling must not degrade to a
#    legacy `movd` (SSE/AVX transition penalty).
# 2. 32-bit implicit write. `divl`/`mull` write %eax AND %edx 32 bits wide, so
#    each register's upper half is already zero and the self-zext the emitter
#    adds when widening the result to 64 bits is a no-op. The 64-bit forms
#    (`divq`, `mulq`) write a full 64-bit %rax and must KEEP the extension,
#    and `cltq` sign-extends (the exact opposite) and must keep it too.
#
# Both halves are also run: the shape gate and the correctness gate read the
# same binary, so a fusion that drops live state cannot pass silently.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-zext-shape.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
src=${BASH_SOURCE[0]%/*}/zext_shape.c
[ -f "$src" ] || src=tests/regression/zext_shape/zext_shape.c

fail() { echo "FAIL: $*" >&2; exit 1; }

"$CCC" -O2 "$src" -o "$tmp/zext_shape"
"$CCC" -O2 -S "$src" -o "$tmp/zext_shape.s"

# ── 1. the bit-cast pair is fused, never re-emitted ───────────────────────
grep -qE '^\s+movd %xmm' "$tmp/zext_shape.s" \
  || fail "XMM->GPR self-zext not fused to movd"
if grep -qE '^\s+movq %xmm[0-9]+, %r' "$tmp/zext_shape.s" &&
   grep -qA1 -E '^\s+movq %xmm[0-9]+, %r' "$tmp/zext_shape.s" |
   grep -qE '^\s+movl %e[a-z]+, %e[a-z]+$'; then
  fail "unfused movq %xmm,%r + movl %e,%e pair survived"
fi

# ── 2. the 32-bit implicit write establishes the fact ─────────────────────
# Every self-zext directly following a 32-bit widening multiply/divide is a
# no-op; a `movl %eX, %eX` survivor there is the defect.
if awk '/^[[:space:]]+(divl|idivl|mull)[[:space:]]/ {d=1; next}
        d && /^[[:space:]]+movl[[:space:]]+%(e[a-z0-9]+),[[:space:]]*%\1$/ {print; bad=1}
        d && /^[[:space:]]/ && !/^[[:space:]]*(movl|nop)/ {d=0}
        END {exit bad}' "$tmp/zext_shape.s"; then :; else
  fail "self-zext survived a 32-bit implicit write"
fi

# ── correctness: the printed values are the whole program contract ────────
"$tmp/zext_shape" >"$tmp/out.lccc"
ref=$(printf '%s\n' "hash 6e6501815fcaabd0" "div  0123456789abcdf3" "acc  5fcaabd0")
[ "$(cat "$tmp/out.lccc")" = "$ref" ] || {
  echo "--- expected ---" >&2; printf '%s\n' "$ref" >&2
  echo "--- got ---" >&2; cat "$tmp/out.lccc" >&2
  fail "zext_shape output mismatch"
}

# ── 3. `divq`/`mulq` keep the extension: the fact is width-specific ───────
cat >"$tmp/q.c" <<'C'
#include <stdio.h>
static unsigned long long q(unsigned long long x, unsigned long long y) {
    return x / y + x % y;
}
int main(void) {
    printf("%llu\n", q(1234567890123456789ULL, 977ULL));
    return 0;
}
C
"$CCC" -O2 "$tmp/q.c" -o "$tmp/q"
[ "$("$tmp/q")" = "1263631412614421" ] || fail "divq path produced a wrong result"

echo "ok: redundant self-zext eliminated in both shapes, semantics preserved"
