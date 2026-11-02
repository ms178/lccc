#!/usr/bin/env bash
# GLA (global location allocation) conservative-policy gate, anchored on the
# real nbody program, plus the default-ON master-gate contract (RA-GLA-02).
#
# nbody's printf format-string global address (v94) spans TWO hole-aware
# live segments around the nested depth-3 simulation loops. Rematerializing
# it reorders the colorer's global FP-loop assignment for +28 instructions
# and +88 stack references (A/B census 2026-09-11), so the shipped
# conservative policy keeps it homed. The rejection used to ride on TWO
# independent fail-closed gates; the pressure profile has since shifted
# (the covered blocks now peak at 15-22 GPR color classes against the
# 12-register budget — measured 2026-09-18 — down from the 38-45 that
# originally pushed every over-budget point outside the default reach
# band), so the reach-band gate now ADMITS the candidate and the ONE-
# SEGMENT CAP is the binding rejector under the default policy. This
# script pins the current facts:
#
#   default (reach=6, cap=1):   the band ADMITS v94 (covers_reachable=
#     true) and the one-segment cap is the binding rejector ("2 segments
#     > 1") — nothing is applied. (History: the 2026-10-12 IVSR recurrence
#     pushed the covered blocks to a 23/12 peak and the band rejected too;
#     since phi incomings are live only on their predecessor edges
#     (liveness.rs, 2026-10-18) the peaks are 13-16/12 and every
#     over-budget block lies inside v94's segments.)
#   reach=999, cap=1:           the band admits at wide reach
#     (covers_reachable=true) but the cap still rejects (the cap is
#     reach-independent);
#   reach=6, cap=2:             both gates admit: exactly one remat (v94,
#     globaladdr) applies, and the assembly is BYTE-IDENTICAL to the
#     default plan — under exact liveness the remat no longer reorders the
#     FP-loop colouring (the +28/+88 above), so the cap costs nothing here
#     and any future divergence fails this cell for re-measurement;
#   reach=0, cap=2:             the band is the binding rejector
#     (`covers_reachable=false`, silent) — nothing is applied, so the band
#     itself keeps deterministic coverage independent of pressure drift;
#   reach=999, cap=2:           BOTH gates open -> exactly one remat
#     applies.
#
# Master gate contract (RA-GLA-02): unset == ON; every documented off token
# (0/off/no/false, case-insensitive, empty) silences the pass; an unset
# variable and =1 produce identical programs; every variant including the
# open-policy one produces output byte-identical to gate-off.
set -eu

# Visible skip when the i386 multilib is absent: CI installs gcc-multilib
# + libc6-dev-i386 and runs the m32 legs for real; a headerless host
# (container sandboxes without root) cannot run them at all, and a hard
# failure there reports the ENVIRONMENT, not the code.
i386_headers_ok() {
    # Rootless hosts can point LCCC_I686_SYSROOT at an unpacked multilib
    # sysroot (the regression suite's convention): exporting it as
    # LCCC_SYSROOT makes both this probe and the i686 legs below run for
    # real instead of skipping. CI installs gcc-multilib and never sets
    # the variable, so its path is unchanged.
    if [ -n "${LCCC_I686_SYSROOT:-}" ] && [ -d "$LCCC_I686_SYSROOT" ]; then
        export LCCC_SYSROOT="$LCCC_I686_SYSROOT"
    fi
    # -E (preprocess-only): the header-visible outcome without linking.
    # (-fsyntax-only is dropped on the stdin input path — the driver links
    # and fails on the empty translation unit's missing main; filed as a
    # follow-up. -E sees exactly the same headers.)
    if ! printf '#include <stdio.h>\n' | "$1" -x c -E - >/dev/null 2>&1; then
        # A host whose own gcc -m32 parses the headers must not skip: the
        # probe failing there is a compiler defect, not a missing sysroot.
        if printf '#include <stdio.h>\n' | gcc -m32 -x c -E - >/dev/null 2>&1; then
            echo "FAIL: i686 probe ($1) cannot parse <stdio.h> though gcc -m32 can" >&2
            exit 1
        fi
        echo "SKIP: i686 leg ($1) - no i386 libc headers on this host (CI installs gcc-multilib)"
        return 1
    fi
}

CCC=${CCC:-./target/fastbuild/lccc}
[ -x "$CCC" ] || { >&2 echo "SKIP: $CCC not built"; exit 1; }
# Canonicalize the compiler path while the caller's cwd is still ours; CI
# passes a relative CCC and this script never cds, but be defensive.
CCC="$(cd "$(dirname "$CCC")" && pwd)/$(basename "$CCC")"
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
src="$dir/../benchmark/programs/nbody.c"
# The master-gate leg needs a TU the default policy edits. zlib_ng_adler32
# stopped firing once phi incomings became edge-live (no block of its main
# exceeds the budget, so remat buys nothing there); the dedicated trampoline
# test is built to stay over budget at its fan-out block.
fire_src="$dir/gla_remat_trampoline.c"
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
    grep -E "$1" "$2" | sed -n '1,3p' | sed 's/^/      /' >&2
    exit 1
  fi
}

# --- default policy: the band admits, the one-segment cap rejects ------
run_trace default
must_grep 'v94' "$tmp/default.log"
must_grep 'v94 .*segments=2 covers_reachable=true' "$tmp/default.log"
must_grep 'rejected: 2 segments > 1' "$tmp/default.log"
must_not_grep '\[GLA\] main applied:' "$tmp/default.log" \
  "default policy edited nbody"

# --- reach wide, segment cap shut: the cap rejects (reach-independent) --
run_trace capshut CCC_GLA_REACH=999
must_grep 'v94 .*segments=2 covers_reachable=true' "$tmp/capshut.log"
must_grep 'rejected: 2 segments > 1' "$tmp/capshut.log"
must_not_grep '\[GLA\] main applied:' "$tmp/capshut.log" \
  "segment cap did not reject nbody"

# --- segment cap open, default reach: one remat, codegen-neutral -------
run_trace bandadmits CCC_GLA_REMAT_MAX_SEGMENTS=2
must_grep 'v94 .*segments=2 covers_reachable=true' "$tmp/bandadmits.log"
must_grep '\[GLA\] main applied: 1 values \(1 remat' "$tmp/bandadmits.log"
cmp -s "$tmp/default.s" "$tmp/bandadmits.s" || {
  >&2 echo "FAIL: the admitted two-segment remat changed nbody's code (pinned: identical to the default plan; re-measure before relaxing the cap)"
  exit 1
}

# --- reach shut, cap open: the BAND is the binding rejector -------------
run_trace bandrejects CCC_GLA_REACH=0 CCC_GLA_REMAT_MAX_SEGMENTS=2
must_grep 'v94 .*segments=2 covers_reachable=false' "$tmp/bandrejects.log"
must_not_grep '\[GLA\] main applied:' "$tmp/bandrejects.log" \
  "reach 0 must not apply a remat"

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
if [ -x "$CCC686" ] && i386_headers_ok "$CCC686"; then
  CCC_RA_GLOBAL_LOCATION=1 CCC_DEBUG_SPLIT=1 CCC_GLA_TRACE=1 \
    "$CCC686" -O2 -S -o "$tmp/m32.s" "$src" 2>"$tmp/m32.log"
  must_not_grep '\[GLA\] main applied:' "$tmp/m32.log" "i686 policy edited nbody"
fi

# --- master gate default ON: a known firing TU applies when the variable
#     is UNSET and silences for every documented off token ----------------
# `CCC_DISABLE_PASSES=ipcp` keeps this fixture's PRESSURE SHAPE, not its code
# quality: every call site passes literals, so interprocedural constant
# propagation (which now also runs before the iteration-0 loop transforms)
# specializes the nine parameters into constants, the live producer set
# collapses, and the planner correctly has nothing to rematerialize. That is a
# win for the program and a blind spot for this gate -- this fixture is the only
# runnable coverage of the critical-edge trampoline emitter, so it pins the
# master gate with specialization held back. The nbody cell above exercises the
# default pipeline end to end.
env -u CCC_RA_GLOBAL_LOCATION CCC_DISABLE_PASSES=ipcp CCC_DEBUG_SPLIT=1 \
  CCC_GLA_TRACE=1 "$CCC" -O2 -S -o "$tmp/fire-default.s" "$fire_src" \
  2>"$tmp/fire-default.log"
must_grep '\[GLA\] select_ptr_64 applied:' "$tmp/fire-default.log" \
  "GLA did not default-ON on gla_remat_trampoline"
CCC_RA_GLOBAL_LOCATION=0 CCC_DISABLE_PASSES=ipcp CCC_DEBUG_SPLIT=1 \
  CCC_GLA_TRACE=1 "$CCC" -O2 -S -o "$tmp/fire-off.s" "$fire_src" \
  2>"$tmp/fire-off.log"
must_not_grep 'applied:' "$tmp/fire-off.log" "=0 did not disable GLA"
for bad in 0 off OFF no NO false FALSE False ""; do
  CCC_RA_GLOBAL_LOCATION="$bad" CCC_DISABLE_PASSES=ipcp CCC_DEBUG_SPLIT=1 \
    CCC_GLA_TRACE=1 "$CCC" -O2 -S -o "$tmp/kill.s" "$fire_src" \
    2>"$tmp/kill.log"
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
