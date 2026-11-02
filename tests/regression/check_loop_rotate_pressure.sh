#!/usr/bin/env bash
# ============================================================================
# check_loop_rotate_pressure.sh — pin loop rotation's Guard G decision.
#
# `loop_rotate_pressure_gate.c` pins rotation's SEMANTICS under pressure (it
# forces the transform on). This script pins the DECISION, which is a
# compile-time property and therefore invisible to a run-and-compare test.
#
# Rotation makes the loop exit reachable from two edges, so each loop-carried
# value that is live out of the loop gains an exit phi and with it a live
# range spanning the whole body. Past the target's spare-register budget that
# costs spills inside the hot loop: measured on `arith_loop` (32 live-out
# ints, x86-64 -O2) the hot loop grew 113 -> 149 instructions and its stack
# traffic 22 -> 47 spill/reloads, for a 24.9% wall-clock loss, while
# `sha256_transform` gained 27.1% from the same pass. Guard G keeps the win
# and drops the loss.
#
# Three assertions, because the first alone would also pass if the loop
# simply never rotated for an unrelated reason:
#   1. high pressure, gate ON  -> assembly identical to rotation disabled
#   2. high pressure, gate OFF -> assembly differs (so (1) is Guard G's doing)
#   3. low  pressure, gate ON  -> assembly differs (rotation still happens)
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
CCC=${CCC:-$here/../../target/fastbuild/lccc}
[[ -x $CCC ]] || { echo "check_loop_rotate_pressure: lccc not found at $CCC" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# One TU per shape. Sharing a TU would couple the two assertions: rotating
# `low_pressure` changes the extracted text of `high_pressure`, and an
# extraction that runs past the first `.size` directive would silently
# compare the wrong function.
src_high="$work/high.c"
src_low="$work/low.c"

cat > "$src_high" <<'EOF'
int high_pressure(int n)
{
    int a = 1, b = 2, c = 3, d = 4, e = 5, f = 6, g = 7, h = 8;
    int i = 9, j = 10, k = 11, l = 12, m = 13, o = 14, p = 15, q = 16;
    for (int it = 0; it < n; it++) {
        a += b * c; b += c * d; c += d * e; d += e * f;
        e += f * g; f += g * h; g += h * i; h += i * j;
        i += j * k; j += k * l; k += l * m; l += m * o;
        m += o * p; o += p * q; p += q * a; q += a * b;
    }
    return a ^ b ^ c ^ d ^ e ^ f ^ g ^ h ^ i ^ j ^ k ^ l ^ m ^ o ^ p ^ q;
}
EOF

cat > "$src_low" <<'EOF'
int low_pressure(int n)
{
    int a = 1, b = 2;
    for (int it = 0; it < n; it++) {
        a += b * 3;
        b += a * 5;
    }
    return a;
}
EOF

# Extract one function's assembly so unrelated codegen cannot mask a change.
emit() { # emit <outfile> <src> <fn> <env assignments...>
  local out=$1 src=$2 fn=$3; shift 3
  env "$@" "$CCC" -O2 -S "$src" -o "$work/full.s"
  awk -v fn="$fn:" '
    $0 == fn        { inf = 1; print; next }
    inf && /^\s*\.size\s/ { print; exit }
    inf             { print }
  ' "$work/full.s" > "$out"
  [[ -s $out ]] || { echo "check_loop_rotate_pressure: no assembly for $fn" >&2; exit 1; }
}

fail=0
note() { printf '%-58s %s\n' "$1" "$2"; }

emit "$work/hp_off.s"  "$src_high" high_pressure CCC_LOOP_ROTATE=
emit "$work/hp_on.s"   "$src_high" high_pressure CCC_LOOP_ROTATE=1
emit "$work/hp_force.s" "$src_high" high_pressure CCC_LOOP_ROTATE=1 CCC_LOOP_ROTATE_IGNORE_PRESSURE=1
emit "$work/lp_off.s"  "$src_low" low_pressure  CCC_LOOP_ROTATE=
emit "$work/lp_on.s"   "$src_low" low_pressure  CCC_LOOP_ROTATE=1

insns() { grep -cE '^[[:space:]]+[a-z]' "$1"; }
stack() { grep -E '^[[:space:]]+[a-z]' "$1" | grep -cE '\(%rsp\)|\(%rbp\)' || true; }

# 1. Gate ON must leave the high-pressure loop byte-identical to no-rotation.
if cmp -s "$work/hp_off.s" "$work/hp_on.s"; then
  note "high pressure + Guard G: identical to rotation-off" "PASS"
else
  note "high pressure + Guard G: identical to rotation-off" \
       "FAIL ($(insns "$work/hp_off.s")/$(stack "$work/hp_off.s") vs $(insns "$work/hp_on.s")/$(stack "$work/hp_on.s") insns/stack)"
  fail=1
fi

# 2. Disabling the gate must change it -- otherwise (1) proved nothing.
if cmp -s "$work/hp_off.s" "$work/hp_force.s"; then
  note "high pressure, gate disabled: rotation actually fires" "FAIL (no change)"
  fail=1
else
  note "high pressure, gate disabled: rotation actually fires" \
       "PASS ($(insns "$work/hp_off.s") -> $(insns "$work/hp_force.s") insns, $(stack "$work/hp_off.s") -> $(stack "$work/hp_force.s") stack)"
fi

# 3. A loop under budget must still rotate, or the gate is a blanket disable.
if cmp -s "$work/lp_off.s" "$work/lp_on.s"; then
  note "low pressure + Guard G: rotation still fires" "FAIL (no change)"
  fail=1
else
  note "low pressure + Guard G: rotation still fires" \
       "PASS ($(insns "$work/lp_off.s") -> $(insns "$work/lp_on.s") insns)"
fi

if ((fail)); then
  echo "loop rotation pressure gate: FAIL"
  exit 1
fi
echo "loop rotation pressure gate: PASS"
