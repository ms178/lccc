#!/usr/bin/env bash
# RA-GLA-03 / structural safety: a source-less global address feeding a loop
# latch phi must NEVER be rematerialized through a BACK edge. Edge
# trampolines exist solely for forward fan-out; splicing one into a latch
# would clone a definition on the hottest edge of a loop nest.
#
# The companion source (gla_backedge_no_trampoline.c) builds a real latch
# phi with the global base redefined on the looping edge and a distinct
# incoming on the forward edge, under heavy register pressure. We force
# EVERY admitting knob maximally open (reach 999, segment cap 1024, weight
# cap 64) and still require:
#   * the GLA trace reports "0 trampolines" on all four targets,
#   * gate-off vs forced-wide binaries emit byte-identical stdout.
# This is the regression encoding of the red-team result that the back-edge
# trampoline shape is unreachable (phi-feeding weight >= 10 > weight cap,
# plus the one-live-segment cap); see engineering/DECISIONS.md RA-GLA-03.
set -euo pipefail
repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
dir=$repo/tests/regression
src=$dir/gla_backedge_no_trampoline.c
x64=$repo/target/fastbuild/lccc
m32=$repo/target/fastbuild/lccc-i686
arm=$repo/target/fastbuild/lccc-arm
rv=$repo/target/fastbuild/lccc-riscv
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# Every admitting knob open at once; the structural gates are what remain.
FORCE=(CCC_DEBUG_SPLIT=1 CCC_RA_GLOBAL_LOCATION=1 CCC_GLA_REACH=999
       CCC_GLA_REMAT_MAX_SEGMENTS=1024 CCC_RA_REMAT_MAX_USES=64 CCC_GLA_TRACE=1)

fail=0
# assert_zero_trampolines LABEL CC INCLUDE OPT
assert_zero_trampolines() {
  local label=$1 cc=$2 inc=$3 opt=$4
  env "${FORCE[@]}" "$cc" ${inc:+-I"$inc"} "$opt" -S "$src" -o "$tmp/$label.s" \
      >"$tmp/$label.trace" 2>&1
  # A nonzero trampoline count (", N trampolines" with N in 1..9) is the
  # failure shape; the applied line must end in "0 trampolines".
  if grep -Eq ', [1-9][0-9]* trampolines' "$tmp/$label.trace"; then
      echo "FAIL: $label $opt produced a back-edge trampoline"; cat "$tmp/$label.trace"; fail=1
  fi
  if grep -q 'applied:' "$tmp/$label.trace" && ! grep -q '0 trampolines' "$tmp/$label.trace"; then
      echo "FAIL: $label $opt trace missing '0 trampolines'"; fail=1
  fi
  echo "ok: $label $opt back-edge trampolines = 0"
}

# Host: the shape survives lowering at -O0 and -O1 (the opts at which GLA
# engages it); -O2/-O3 must never show a nonzero count either.
[ -x "$x64" ] || { echo "SKIP: $x64 missing"; exit 0; }
for opt in -O0 -O1 -O2 -O3; do
  assert_zero_trampolines "x64" "$x64" "$(gcc -print-file-name=include)" "$opt"
done

# Cross targets: trace-level pin (no execution needed); skip absent bins.
if [ -x "$arm" ] && command -v aarch64-linux-gnu-gcc >/dev/null 2>&1; then
  assert_zero_trampolines "aarch64" "$arm" \
    "$(aarch64-linux-gnu-gcc -print-file-name=include)" -O1
fi
if [ -x "$rv" ] && command -v riscv64-linux-gnu-gcc >/dev/null 2>&1; then
  assert_zero_trampolines "riscv64" "$rv" \
    "$(riscv64-linux-gnu-gcc -print-file-name=include)" -O1
fi
if [ -x "$m32" ]; then
  assert_zero_trampolines "i686" "$m32" "" -O1
fi

# Runtime equivalence: forcing every knob open must not change the result.
CCC_RA_GLOBAL_LOCATION=0 "$x64" -O1 "$src" -o "$tmp/off.bin" 2>/dev/null
env "${FORCE[@]}" "$x64" -O1 "$src" -o "$tmp/on.bin" 2>/dev/null
"$tmp/off.bin" > "$tmp/off.out"; "$tmp/on.bin" > "$tmp/on.out"
if ! cmp -s "$tmp/off.out" "$tmp/on.out"; then
  echo "FAIL: forced-wide vs gate-off output differs"; fail=1
else
  echo "ok: forced-wide output == gate-off ($(cat "$tmp/off.out"))"
fi

if [ "$fail" -ne 0 ]; then exit 1; fi
echo "back-edge no-trampoline pin passes"
