#!/usr/bin/env bash
# GLA (global location allocation) conservative-policy gate, anchored on the
# real nbody program.
#
# nbody's printf format-string global address (v94) spans TWO hole-aware
# live segments around the nested depth-3 simulation loops, and every block
# it covers peaks at 38-45 GPR color classes against a 12-register budget.
# Rematerializing it reorders the colorer's global FP-loop assignment for
# +28 instructions and +88 stack references (A/B census 2026-09-11). The
# shipped conservative policy therefore rejects it through TWO independent
# fail-closed gates, and this script pins both plus their conjunction:
#
#   default (reach=6, cap=1): v94 covers no *reachable* over-budget point
#     (block-level reach-band gate) and nothing is applied;
#   reach=999, cap=1:        the candidate becomes reachable but the
#     one-segment cap rejects it ("2 segments > 1");
#   reach=6, cap=2:         the segment cap is open but the reach-band gate
#     keeps covers_reachable=false and nothing is applied;
#   reach=999, cap=2:       BOTH gates open -> exactly one remat applies.
#
# Every configuration (including the applied one) must produce output
# byte-identical to the GLA-off build (soundness, not just sizing).
set -eu

CCC=${CCC:-./target/fastbuild/lccc}
# Canonicalize the compiler path while the caller's cwd is still ours; CI
# passes a relative CCC and this script never cds, but be defensive.
if [ -x "$CCC" ]; then
  CCC="$(cd "$(dirname "$CCC")" && pwd)/$(basename "$CCC")"
fi
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
src="$dir/../benchmark/programs/nbody.c"
tmp="${TMPDIR:-/tmp}/lccc-gla-remat.$$"
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
mkdir -p "$tmp"

[ -x "$CCC" ] || { >&2 echo "SKIP: $CCC not built"; exit 1; }

run_trace() { # $1 = log name; remaining args = extra NAME=VALUE env
  local log="$1"; shift
  # `env` (not bash assignment prefixes): env words from "$@" must still be
  # recognized as NAME=VALUE, which simple-command expansion never does.
  env CCC_RA_GLOBAL_LOCATION=1 CCC_DEBUG_SPLIT=1 CCC_GLA_TRACE=1 "$@" \
    "$CCC" -O2 -S -o "$tmp/$log.s" "$src" 2>"$tmp/$log.log"
}

# --- default policy: reach-band gate rejects the hopeless blocks --------
run_trace default
grep -q 'v94' "$tmp/default.log"
grep -Eq 'v94 .*segments=2 covers_reachable=false' "$tmp/default.log"
! grep -Eq '\[GLA\] main applied:' "$tmp/default.log"

# --- reach wide, segment cap shut: cap rejects --------------------------
run_trace capshut CCC_GLA_REACH=999
grep -Eq 'v94 .*segments=2 covers_reachable=true' "$tmp/capshut.log"
grep -Eq 'rejected: 2 segments > 1' "$tmp/capshut.log"
! grep -Eq '\[GLA\] main applied:' "$tmp/capshut.log"

# --- segment cap open, reach shut: reach gate rejects -------------------
run_trace reachshut CCC_GLA_REMAT_MAX_SEGMENTS=2
grep -Eq 'v94 .*segments=2 covers_reachable=false' "$tmp/reachshut.log"
! grep -Eq '\[GLA\] main applied:' "$tmp/reachshut.log"

# --- both gates open: the candidate applies -----------------------------
run_trace open CCC_GLA_REACH=999 CCC_GLA_REMAT_MAX_SEGMENTS=2
grep -Eq '\[GLA\] main applied: 1 values \(1 remat' "$tmp/open.log"

# --- size tiers plan nothing (remats trade carried homes for per-cluster
#     instructions against the -Os objective; see Tier::Size) --------------
CCC_RA_GLOBAL_LOCATION=1 CCC_DEBUG_SPLIT=1 CCC_GLA_TRACE=1 \
  "$CCC" -Os -S -o "$tmp/os.s" "$src" 2>"$tmp/os.log"
! grep -Eq '\[GLA\] main applied:' "$tmp/os.log"

# --- i686 speed tier: the narrow reach band plus net-relief credit leave
#     nbody entirely alone; its 3-use globals would otherwise steal %esi
#     from an induction counter (measured loop_patterns 1.05x regression
#     at the old band 6) ---------------------------------------------------
CCC686="$dir/../../target/fastbuild/lccc-i686"
if [ -x "$CCC686" ]; then
  CCC_RA_GLOBAL_LOCATION=1 CCC_DEBUG_SPLIT=1 CCC_GLA_TRACE=1 \
    "$CCC686" -O2 -S -o "$tmp/m32.s" "$src" 2>"$tmp/m32.log"
  ! grep -Eq '\[GLA\] main applied:' "$tmp/m32.log"
fi

# --- soundness: all variants, and GLA-off, produce identical output ------
"$CCC" -O2 "$src" -o "$tmp/off.bin" 2>/dev/null
CCC_RA_GLOBAL_LOCATION=1 "$CCC" -O2 "$src" -o "$tmp/on.bin" 2>/dev/null
CCC_RA_GLOBAL_LOCATION=1 CCC_GLA_REACH=999 CCC_GLA_REMAT_MAX_SEGMENTS=2 \
  "$CCC" -O2 "$src" -o "$tmp/open.bin" 2>/dev/null
"$tmp/off.bin" >"$tmp/off.out"
"$tmp/on.bin" >"$tmp/on.out"
"$tmp/open.bin" >"$tmp/open.out"
cmp -s "$tmp/off.out" "$tmp/on.out"
cmp -s "$tmp/off.out" "$tmp/open.out"
# Size tier on/off must also be output-identical.
"$CCC" -Os "$src" -o "$tmp/osoff.bin" 2>/dev/null
CCC_RA_GLOBAL_LOCATION=1 "$CCC" -Os "$src" -o "$tmp/oson.bin" 2>/dev/null
"$tmp/osoff.bin" >"$tmp/osoff.out"
"$tmp/oson.bin" >"$tmp/oson.out"
cmp -s "$tmp/osoff.out" "$tmp/oson.out"

echo "OK check_gla_remat_policy"
