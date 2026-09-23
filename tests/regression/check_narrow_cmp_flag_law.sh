#!/usr/bin/env bash
# Asm-level pins for the narrow-compare fold's SF/OF flag-law window
# (fold_narrow_load_imm_compare).
#
# The fold `movzbl SLOT, %R; cmpl $I, %R` -> `cmpb $I, SLOT` preserves ZF
# and CF only.  The flag-reader window must therefore
#   (a) REFUSE the fold when any flags reader in the window is an
#       SF/OF consumer (signed orderings jl/jg/jle/jge, sets/setl/setg/
#       setle/setge, js/jns) — the historical `setg`-after-fold miscompile
#       (u8 c > 50 with c = 200 returned 0), including a dangerous reader
#       BEHIND a safe one (`cmp; jb X; jg Y`);
#   (b) still APPLY the fold for ZF/CF readers (je/jne, jb/jbe/ja/jae +
#       setcc twins) — so the window does not over-reject the shape the
#       fold exists for.
#
# The execution-level semantics live in
# tests/regression/i686_narrow_cmp_flag_law.c; this gate pins the emitted
# shapes so a "fix" that deletes the fold entirely (safe but slow) or a
# window that is too narrow (fast but wrong) both fail here.
#
# Usage:
#   tests/regression/check_narrow_cmp_flag_law.sh
# Environment:
#   LCCC  compiler to test (default: target/fastbuild/lccc)
set -uo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repo_root"

LCCC=${LCCC:-${CCC:-target/fastbuild/lccc}}
[[ "$LCCC" == /* ]] || LCCC="$repo_root/$LCCC"

workdir=$(mktemp -d)
trap 'rm -rf "$workdir"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "ok: $*"; }

cat > "$workdir/src.c" <<'EOF'
typedef unsigned int uint32_t;
typedef unsigned char uint8_t;
__attribute__((noinline)) uint32_t sgt(uint32_t c) { return (uint8_t)c > 50; }
__attribute__((noinline)) uint32_t ult(uint32_t c) { return (uint8_t)c < 50u; }
__attribute__((noinline)) uint32_t seq(uint32_t c) { return (uint8_t)c == 50; }
EOF

"$LCCC" -m32 -Os -mregparm=3 -fno-pic -S -o "$workdir/out.s" "$workdir/src.c" \
    || fail "lccc failed to compile the probe"

sgt_body=$(awk '/^sgt:/,/^$/' "$workdir/out.s" | head -40)
ult_body=$(awk '/^ult:/,/^$/' "$workdir/out.s" | head -40)
seq_body=$(awk '/^seq:/,/^$/' "$workdir/out.s" | head -40)

[[ -n "$sgt_body" ]] || fail "no sgt body in the asm"
[[ -n "$ult_body" ]] || fail "no ult body in the asm"
[[ -n "$seq_body" ]] || fail "no seq body in the asm"

# (a) the signed-ordering consumer must stay WIDE: no cmpb $50 in sgt.
if grep -q "cmpb \$50" <<<"$sgt_body"; then
    fail "sgt was narrowed despite the signed reader (the setg/jg miscompile):
$sgt_body"
fi
# and the wide compare + signed reader pair must be present.
grep -q "cmpl \$50" <<<"$sgt_body" \
    || fail "sgt lost its wide compare:
$sgt_body"
pass "signed-ordering reader keeps the wide compare (no cmpb in sgt)"

# (b) the CF-safe reader must still fold to the narrow form.
grep -q "cmpb \$50" <<<"$ult_body" \
    || fail "ult (jb reader) was NOT folded — the window over-rejects:
$ult_body"
pass "CF-safe reader still folds (cmpb present in ult)"

# (b2) the ZF-safe reader must still fold.
grep -q "cmpb \$50" <<<"$seq_body" \
    || fail "seq (je reader) was NOT folded — the window over-rejects:
$seq_body"
pass "ZF-safe reader still folds (cmpb present in seq)"

# (c) kill switch still disables the fold entirely (the ult shape reverts).
CCC_NO_NARROW_CMP_FOLD=1 "$LCCC" -m32 -Os -mregparm=3 -fno-pic -S \
    -o "$workdir/out_killed.s" "$workdir/src.c" \
    || fail "lccc (kill switch) failed to compile the probe"
ult_killed=$(awk '/^ult:/,/^$/' "$workdir/out_killed.s" | head -40)
grep -q "cmpb \$50" <<<"$ult_killed" \
    || fail "kill switch did not revert the fold:
$ult_killed"
grep -q "movzbl" <<<"$ult_killed" \
    || fail "kill switch output lost the zero-extend load:
$ult_killed"
pass "kill switch reverts the fold"

echo "PASS: check_narrow_cmp_flag_law"
