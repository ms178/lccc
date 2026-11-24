#!/usr/bin/env bash
# The i686 fused `res * base + val` reads an 8- or 16-bit operand narrow.
#
# A 64-bit `res = res * base + val` is one sequence on i686, and the cast
# that widens `val` or `base` emits nothing: the sequence reads the operand
# where it lives. It read all 32 bits of that home, and narrow arithmetic
# is not cut back in its home, so `unsigned short n = 0; n--; sum = sum * 31
# + n;` added 0xffffffff for 0xffff (tests/regression/mulacc_narrow_feeder.c
# has the story and the cases). The sequence now extends a narrow addend
# itself and reads a narrow base from the slot its cast wrote.
#
#   1. Run: every recurrence of the test agrees with the same recurrence
#      whose operand went through memory, at -O0, -O1, -O2, -O3 and -Os.
#      The program says so by its exit status. (Before the fix it failed
#      at -O2, -O3 and -Os.)
#   2. Read: the fix is not that the fusion was given up. At -O2 no chain is
#      rejected, each function with a narrow addend extends it into %ecx and
#      adds that, and none adds a callee-saved register to the low half as
#      it stood, which is what the defect looked like.
set -euo pipefail
cd "$(dirname "$0")/../.."
ccc=${CCC_I686:-target/fastbuild/lccc-i686}
[[ -x $ccc ]] || { echo "check_i686_mulacc_narrow_feeder: no i686 driver at $ccc" >&2; exit 1; }
src=tests/regression/mulacc_narrow_feeder.c
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fail=0
ok()  { printf 'ok:   %s\n' "$1"; }
bad() { printf 'FAIL: %s\n' "$1"; fail=1; }

# ── 1. Run ──────────────────────────────────────────────────────────────────
for opt in -O0 -O1 -O2 -O3 -Os; do
  "$ccc" "$opt" "$src" -o "$work/run"
  if timeout 10 "$work/run" > "$work/out" 2>&1; then
    ok "$opt: as written and through memory agree"
  else
    bad "$opt: as written and through memory differ"
    grep -A1 'DIFFERS' "$work/out" || cat "$work/out"
  fi
done

# ── 2. Read ─────────────────────────────────────────────────────────────────
CCC_DEBUG_MULACC=1 "$ccc" -O2 -S "$src" -o "$work/fused.s" 2> "$work/trace"
count() { grep -cE -- "$1" "$2" || true; }
if (($(count 'FUSED' "$work/trace") > 0)) && (($(count 'REJECTED' "$work/trace") == 0)); then
  ok "-O2: the chains are fused, none rejected"
else
  bad "-O2: fused $(count 'FUSED' "$work/trace"), rejected $(count 'REJECTED' "$work/trace")"
fi

# The instructions of one function.
body_of() {
  awk -v fn="$1:" '
    $0 == fn { inf = 1; next }
    inf && /^[[:space:]]*\.size[[:space:]]/ { exit }
    inf { print }' "$work/fused.s"
}
for fn in u8_up u8_down u16_up u16_down i8_up i8_down i16_up i16_down u8_by_100 i16_by_9000; do
  body_of "addend_fused_$fn" > "$work/body"
  extended=$(count '^[[:space:]]+mov[sz][bw]l[[:space:]].*,[[:space:]]*%ecx$' "$work/body")
  added=$(count '^[[:space:]]+addl[[:space:]]+%ecx,[[:space:]]*%eax$' "$work/body")
  raw=$(count '^[[:space:]]+addl[[:space:]]+%(ebx|esi|edi|ebp),[[:space:]]*%eax$' "$work/body")
  if ((extended >= 1 && added >= 1 && raw == 0)); then
    ok "addend_fused_$fn: the addend is extended, then added"
  else
    bad "addend_fused_$fn: extended $extended, added $added, added as it stood $raw"
  fi
done
# A narrow base is not read from a register: the multiply takes the slot
# its cast wrote, or an immediate.
for fn in u8_up u8_down u16_up u16_down; do
  body_of "base_fused_$fn" > "$work/body"
  raw=$(count '^[[:space:]]+mull[[:space:]]+%(ebx|esi|edi|ebp)$' "$work/body")
  mul=$(count '^[[:space:]]+mull[[:space:]]' "$work/body")
  if ((mul >= 1 && raw == 0)); then
    ok "base_fused_$fn: the base is not multiplied as it stood"
  else
    bad "base_fused_$fn: $mul multiplications, $raw by a register as it stood"
  fi
done

if ((fail)); then
  echo "i686 mul-accumulate narrow operands: FAIL"
  exit 1
fi
echo "i686 mul-accumulate narrow operands: PASS"
