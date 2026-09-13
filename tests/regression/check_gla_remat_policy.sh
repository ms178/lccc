#!/usr/bin/env bash
# GLA (global location allocation) conservative-policy gate, anchored on the
# real nbody program, plus the default-ON master-gate contract (RA-GLA-02).
#
# nbody's printf format-string global address (v94) spans TWO hole-aware
# live segments around the nested depth-3 simulation loops, and every block
# it covers peaks at 38-45 GPR color classes against a 12-register budget.
# Rematerializing it reorders the colorer's global FP-loop assignment for
# +28 instructions and +88 stack references (A/B census 2026-09-11). The
# shipped conservative policy rejects it through TWO independent
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
# Master gate contract (RA-GLA-02): unset == ON; every documented off token
# (0/off/no/false, case-insensitive, empty) silences the pass; an unset
# variable and =1 produce identical programs; every variant including the
# open-policy one produces output byte-identical to gate-off.
set -eu

CCC=${CCC:-./target/fastbuild/lccc}
[ -x "$CCC" ] || { >&2 echo "SKIP: $CCC not built"; exit 1; }
# Canonicalize the compiler path while the caller's cwd is still ours; CI
# passes a relative CCC and this script never cds, but be defensive.
CCC="$(cd "$(dirname "$CCC")" && pwd)/$(basename "$CCC")"
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
src="$dir/../benchmark/programs/nbody.c"
fire_src="$dir/../benchmark/programs/zlib_ng_adler32.c"
tmp="$(mktemp -d "${TMPDIR:-/tmp}/lccc-gla-remat.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

run_trace() { # $1 = log name; remaining args = extra NAME=VALUE env
  local log="$1"; shift
  # `env` (not bash assignment prefixes): env words from "$@" must still be
  # recognized as NAME=VALUE, which simple-command expansion never does.
  env CCC_RA_GLOBAL_LOCATION=1 CCC_DEBUG_SPLIT=1 CCC_GLA_TRACE=1 "$@" \
    "$CCC" -O2 -S -o "$tmp/$log.s" "$src" 2>"$tmp/$log.log"
}

# Assertion helpers. A bare `! grep ...` is INERT under `set -e` (negated
# commands are exempt from errexit), so absence must be checked explicitly.
must_grep() { # must_grep PAT FILE [description]
  grep -Eq "$1" "$2" || { >&2 echo "FAIL: ${3:-expected pattern missing: $1 in $2}"; exit 1; }
}
must_not_grep() { # must_not_grep PAT FILE [description]
  if grep -Eq "$1" "$2"; then
    >&2 echo "FAIL: ${3:-forbidden pattern present: $1 in $2}"
    grep -E "$1" "$2" | head -3 | sed 's/^/      /' >&2
    exit 1
  fi
}

# --- default policy: reach-band gate rejects the hopeless blocks --------
run_trace default
must_grep 'v94' "$tmp/default.log"
must_grep 'v94 .*segments=2 covers_reachable=false' "$tmp/default.log"
must_not_grep '\[GLA\] main applied:' "$tmp/default.log" \
  "default policy edited nbody"

# --- reach wide, segment cap shut: cap rejects --------------------------
run_trace capshut CCC_GLA_REACH=999
must_grep 'v94 .*segments=2 covers_reachable=true' "$tmp/capshut.log"
must_grep 'rejected: 2 segments > 1' "$tmp/capshut.log"
must_not_grep '\[GLA\] main applied:' "$tmp/capshut.log" \
  "segment cap did not reject nbody"

# --- segment cap open, reach shut: reach gate rejects -------------------
run_trace reachshut CCC_GLA_REMAT_MAX_SEGMENTS=2
must_grep 'v94 .*segments=2 covers_reachable=false' "$tmp/reachshut.log"
must_not_grep '\[GLA\] main applied:' "$tmp/reachshut.log" \
  "reach gate did not reject nbody"

# --- both gates open: the candidate applies -----------------------------
run_trace open CCC_GLA_REACH=999 CCC_GLA_REMAT_MAX_SEGMENTS=2
must_grep '\[GLA\] main applied: 1 values \(1 remat' "$tmp/open.log"

# --- size tiers plan nothing (remats trade carried homes for per-cluster
#     instructions against the -Os objective; see Tier::Size) --------------
CCC_RA_GLOBAL_LOCATION=1 CCC_DEBUG_SPLIT=1 CCC_GLA_TRACE=1 \
  "$CCC" -Os -S -o "$tmp/os.s" "$src" 2>"$tmp/os.log"
must_not_grep '\[GLA\] main applied:' "$tmp/os.log" "-Os edited nbody"

# --- i686 speed tier: the narrow reach band plus net-relief credit leave
#     nbody entirely alone; its 3-use globals would otherwise steal %esi
#     from an induction counter (measured loop_patterns 1.05x regression
#     at the old band 6). The i686 driver is the argv0-selected bin. --------
CCC686="$dir/../../target/fastbuild/lccc-i686"
if [ -x "$CCC686" ]; then
  CCC_RA_GLOBAL_LOCATION=1 CCC_DEBUG_SPLIT=1 CCC_GLA_TRACE=1 \
    "$CCC686" -O2 -S -o "$tmp/m32.s" "$src" 2>"$tmp/m32.log"
  must_not_grep '\[GLA\] main applied:' "$tmp/m32.log" "i686 policy edited nbody"
fi

# --- master gate default ON: a known firing TU applies when the variable
#     is UNSET and silences for every documented off token ----------------
env -u CCC_RA_GLOBAL_LOCATION CCC_DEBUG_SPLIT=1 CCC_GLA_TRACE=1 \
  "$CCC" -O2 -S -o "$tmp/fire-default.s" "$fire_src" 2>"$tmp/fire-default.log"
must_grep '\[GLA\] main applied:' "$tmp/fire-default.log" \
  "GLA did not default-ON on adler32"
CCC_RA_GLOBAL_LOCATION=0 CCC_DEBUG_SPLIT=1 CCC_GLA_TRACE=1 \
  "$CCC" -O2 -S -o "$tmp/fire-off.s" "$fire_src" 2>"$tmp/fire-off.log"
must_not_grep 'applied:' "$tmp/fire-off.log" "=0 did not disable GLA"
for bad in 0 off OFF no NO false FALSE False ""; do
  CCC_RA_GLOBAL_LOCATION="$bad" CCC_DEBUG_SPLIT=1 CCC_GLA_TRACE=1 \
    "$CCC" -O2 -S -o "$tmp/kill.s" "$fire_src" 2>"$tmp/kill.log"
  must_not_grep 'applied:' "$tmp/kill.log" "off token '$bad' did not disable GLA"
done

# --- soundness: default/=1/open/off variants all produce identical output
CCC_RA_GLOBAL_LOCATION=0 "$CCC" -O2 "$src" -o "$tmp/off.bin" 2>/dev/null
CCC_RA_GLOBAL_LOCATION=1 "$CCC" -O2 "$src" -o "$tmp/on.bin" 2>/dev/null
# Unset variable follows the default: must match =1 byte-for-byte.
env -u CCC_RA_GLOBAL_LOCATION "$CCC" -O2 "$src" -o "$tmp/default.bin" 2>/dev/null
CCC_RA_GLOBAL_LOCATION=1 CCC_GLA_REACH=999 CCC_GLA_REMAT_MAX_SEGMENTS=2 \
  "$CCC" -O2 "$src" -o "$tmp/open.bin" 2>/dev/null
"$tmp/off.bin" >"$tmp/off.out"
"$tmp/on.bin" >"$tmp/on.out"
"$tmp/default.bin" >"$tmp/default.out"
"$tmp/open.bin" >"$tmp/open.out"
cmp -s "$tmp/off.out" "$tmp/on.out" || { >&2 echo "FAIL: on != off"; exit 1; }
cmp -s "$tmp/off.out" "$tmp/open.out" || { >&2 echo "FAIL: open != off"; exit 1; }
cmp -s "$tmp/default.out" "$tmp/on.out" || { >&2 echo "FAIL: default != on"; exit 1; }

# Size tier on/off must also be output-identical.
CCC_RA_GLOBAL_LOCATION=0 "$CCC" -Os "$src" -o "$tmp/osoff.bin" 2>/dev/null
CCC_RA_GLOBAL_LOCATION=1 "$CCC" -Os "$src" -o "$tmp/oson.bin" 2>/dev/null
"$tmp/osoff.bin" >"$tmp/osoff.out"
"$tmp/oson.bin" >"$tmp/oson.out"
cmp -s "$tmp/osoff.out" "$tmp/oson.out" || { >&2 echo "FAIL: -Os on != off"; exit 1; }

echo "OK check_gla_remat_policy"
