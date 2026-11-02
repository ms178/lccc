#!/usr/bin/env bash
# CCC_IVSR_SCALAR_DERIVED default-pin gate (audit F5).
#
# The scalar derived-IV flavor of iv_strength_reduce measured as a net
# LOSS (linux_rbtree +1.2%, expat +4%, glibc_strstr +4.3%, five golden
# workloads past the codegen-quality bands — see iv_strength_reduce.rs)
# and is therefore OPT-IN via CCC_IVSR_SCALAR_DERIVED=1.  An opt-in knob
# whose default silently flips back ON would reintroduce a measured
# regression with nothing to catch it: the knob has no natural presence
# in any other gate (the S46 post-mortem removed its shape pin precisely
# because pinning a regression's shape is wrong).
#
# This gate pins the default three ways, all without pinning any
# particular asm shape:
#   A. DEFAULT differential: -O2 linux_rbtree vs gcc, bit-exact.
#   B. OPT-IN differential:  CCC_IVSR_SCALAR_DERIVED=1 -O2 vs gcc,
#      bit-exact — the opt-in path stays CORRECT even though it is the
#      slower shape.
#   C. THE DEFAULT PIN: the knob is default-OFF *and* live — the default
#      build and the opt-in build must produce DIFFERENT assembly for the
#      derived-IV workload.  Identical asm means either the default
#      flipped on (the measured regression is back) or the machinery
#      died (the knob is a no-op nobody noticed rotting).
set -u
# GCC availability: this is a CORRECTNESS gate (differentials A/B), so a
# missing/broken reference compiler must FAIL, not silently skip both
# comparisons. Set LCCC_ALLOW_GCC_SKIP=1 only for exotic environments
# without any GCC (explicit opt-out, uniform across regression gates;
# CI never sets it).
CCC=${CCC:-target/fastbuild/lccc}
RB=$(dirname "$0")/../benchmark/programs/linux_rbtree.c
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
GCC=${GCC_BIN:-gcc}
GCCINC=$("$GCC" -print-file-name=include 2>/dev/null)
[ -d "$GCCINC" ] && GCCINC="-I$GCCINC" || GCCINC=""
fails=0
note() { printf '%s\n' "$*"; }
bad()  { note "FAIL: $*"; fails=$((fails+1)); }

# ── A + B. runtime differentials (default and opt-in) ────────────────
for MODE in default optin; do
  if [ "$MODE" = default ]; then
    ENVBIN=""
    TAG="default (knob unset)"
  else
    ENVBIN="CCC_IVSR_SCALAR_DERIVED=1"
    TAG="opt-in (CCC_IVSR_SCALAR_DERIVED=1)"
  fi
  "$GCC" -O2 -o "$TMP/g_$MODE.x" "$RB" 2>/dev/null \
    || { if [ "$ALLOW_GCC_SKIP" = 1 ]; then note "skip $TAG (gcc build; LCCC_ALLOW_GCC_SKIP=1)"; continue; else bad "$TAG gcc build failed (no silent skip: set LCCC_ALLOW_GCC_SKIP=1 to opt out)"; continue; fi; }
  if [ -n "$ENVBIN" ]; then
    env $ENVBIN "$CCC" $GCCINC -O2 -o "$TMP/l_$MODE.x" "$RB" 2>/dev/null \
      || { bad "$TAG lccc compile"; continue; }
  else
    # The default arm MUST force the default: an inherited exported
    # CCC_IVSR_SCALAR_DERIVED would otherwise make this "default" build
    # secretly opt-in (and the C pin below would compare opt-in vs opt-in).
    env -u CCC_IVSR_SCALAR_DERIVED "$CCC" $GCCINC -O2 -o "$TMP/l_$MODE.x" "$RB" 2>/dev/null \
      || { bad "$TAG lccc compile"; continue; }
  fi
  G=$("$TMP/g_$MODE.x"; echo "rc=$?")
  L=$("$TMP/l_$MODE.x"; echo "rc=$?")
  [ "$G" = "$L" ] || bad "$TAG runtime differs (gcc=[$G] lccc=[$L])"
done
note "A/B. runtime differentials done (fails so far: $fails)"

# ── C. the default pin: knob unset vs set must DIFFER in asm ─────────
# (default arm forces the knob unset — see above — so ambient exports
# cannot make this compare opt-in against itself).
env -u CCC_IVSR_SCALAR_DERIVED "$CCC" $GCCINC -O2 -S "$RB" -o "$TMP/def.s" 2>/dev/null \
  || bad "default -S compile"
env CCC_IVSR_SCALAR_DERIVED=1 "$CCC" $GCCINC -O2 -S "$RB" -o "$TMP/opt.s" 2>/dev/null \
  || bad "opt-in -S compile"
if [ -f "$TMP/def.s" ] && [ -f "$TMP/opt.s" ]; then
  if cmp -s "$TMP/def.s" "$TMP/opt.s"; then
    bad "default pin: knob-unset and knob-set asm are IDENTICAL — either the \
scalar derived-IV default flipped back ON (the measured regression is back) \
or the knob is dead (machinery rotted with no signal)"
  else
    note "C. default pin: knob is opt-in and live (asm differs as required)"
  fi
fi

if [ "$fails" -eq 0 ]; then
  note "ivsr scalar-derived default-pin gate: PASS"
  exit 0
fi
note "ivsr scalar-derived default-pin gate: $fails FAILURES"
exit 1
