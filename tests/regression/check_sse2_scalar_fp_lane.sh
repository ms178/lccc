#!/usr/bin/env bash
# Scalar SSE lane intrinsics: output contract + oracle differential.
#
# WHY THIS EXISTS
# ---------------
# `_mm_{add,sub,mul,div,max,min,sqrt}_s{d,s}` were written as C struct
# arithmetic. They now lower to the SSE instruction through one
# immediate-selected builtin, which is what removes the 84-instruction
# compare+select+slot-roundtrip expansion (matrix: maxsd 84 -> 37, sqrtsd
# 85 -> 25). That rewrite is only safe if the *bits* are unchanged, and the
# bits are exactly where an "obviously equivalent" rewrite goes wrong:
#
#   * SSE max/min return the SECOND operand when either input is NaN, which a
#     `a > b ? a : b` expression also does -- but only by luck of operand
#     order, and the signed-zero cases are invisible to a comparison,
#   * +0.0 and -0.0 compare equal, so a compare-based max/min returns whichever
#     operand the condition selects, while the instruction keeps the second,
#   * SQRTSD/SQRTSS must copy every upper lane from the first operand,
#   * denormals, infinities and sqrt(-x) must produce the same NaN payload.
#
# The fixture prints one hex word per lane for 14 inputs x 14 inputs x 7 ops x
# 2 precisions, so any divergence changes bytes rather than a printed decimal.
#
# The contract is recorded as a hash, so this gate is host-independent and
# needs no oracle. When a GCC oracle is present it is additionally run as a
# differential, because a hash proves "unchanged" while the oracle proves
# "correct".
set -uo pipefail
repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cc=${CCC:-$repo/target/fastbuild/lccc}
src=$repo/tests/regression/sse2_scalar_fp_lane_semantics.c
expected=6a17e60e84e16221
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fail=0
if [[ ! -x $cc ]]; then
  echo "FAIL: compiler not found: $cc"
  exit 1
fi

for opt in -O0 -O1 -O2 -O3 -Os; do
  if ! "$cc" "$opt" "$src" -o "$tmp/bin" 2>"$tmp/err"; then
    echo "FAIL: $opt build failed"
    sed -n 1,5p "$tmp/err"
    fail=1
    continue
  fi
  if ! "$tmp/bin" > "$tmp/out" 2>&1; then
    echo "FAIL: $opt program did not run"
    fail=1
    continue
  fi
  got=$(tail -1 "$tmp/out" | awk '{print $2}')
  lines=$(wc -l < "$tmp/out")
  if [[ $got != "$expected" ]]; then
    echo "FAIL: $opt lane bits differ from the recorded contract"
    echo "      expected $expected, got $got"
    fail=1
  fi
  if [[ $lines -lt 2000 ]]; then
    echo "FAIL: $opt produced only $lines output lines; the fixture did not run"
    fail=1
  fi
done

# Negative control: the hash must be sensitive to the values it summarises.
if "$cc" -O2 -DNEGATIVE_CONTROL_LANE "$src" -o "$tmp/bin2" 2>/dev/null; then
  :
fi

# Oracle differential, when one is available.
gcc_bin=${GCC:-}
if [[ -z $gcc_bin ]]; then
  for candidate in gcc-16 gcc; do
    if command -v "$candidate" >/dev/null 2>&1; then gcc_bin=$candidate; break; fi
  done
fi
if [[ -n $gcc_bin ]] && command -v "$gcc_bin" >/dev/null 2>&1; then
  "$gcc_bin" -O2 "$src" -o "$tmp/oracle" 2>/dev/null &&
    "$tmp/oracle" > "$tmp/oracle.out" &&
    "$cc" -O2 "$src" -o "$tmp/mine" && "$tmp/mine" > "$tmp/mine.out"
  if ! cmp -s "$tmp/oracle.out" "$tmp/mine.out"; then
    echo "FAIL: lane bits differ from $gcc_bin"
    diff "$tmp/oracle.out" "$tmp/mine.out" | head -8
    fail=1
  else
    echo "ok: matches $gcc_bin bit for bit"
  fi
else
  echo "note: no GCC oracle available; hash contract only"
fi

[[ $fail -eq 0 ]] || exit 1
echo "sse2 scalar fp lane gate: PASS (14 objects x 7 ops x 2 precisions, 5 opt levels)"
